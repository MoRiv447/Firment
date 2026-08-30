//! `observe` — read-only computer-vision analysis of frames (photos of the
//! target), giving the agent evidence at rung 5 of the verification ladder:
//! "the device physically behaves as asked". Phase 1 is brightness/lit
//! analysis of a still image; blink/motion/diff arrive in a later release.
//!
//! Deterministic local CV via the `image` crate — no vision model, no
//! provider changes, results are reproducible and unit-testable.

use super::util::{resolve_within, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use image::RgbaImage;
use serde_json::{Value, json};

pub struct Observe;

/// Observation region, [x, y, w, h].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Source of frames to analyze. A trait so tests can inject synthetic frames
/// (the `FakeReader` move from monitor.rs) and so the later camera/command
/// capture paths slot in without touching the analyzers.
pub trait FrameSource {
    fn next_frame(&mut self) -> Result<Option<RgbaImage>, String>;
}

/// Phase-1 source: one still image from the workspace.
pub struct StillSource {
    pub frame: RgbaImage,
}

impl FrameSource for StillSource {
    fn next_frame(&mut self) -> Result<Option<RgbaImage>, String> {
        Ok(Some(self.frame.clone()))
    }
}

/// What the analyzer is asked to measure.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// None = the whole frame.
    pub roi: Option<Rect>,
    /// None = derived from the frame's own min/max midpoint.
    pub threshold: Option<u8>,
}

/// How sure the analyzer is about the lit verdict — low confidence MUST be
/// surfaced as such so the model cannot over-assert physical behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn name(self) -> &'static str {
        match self {
            Confidence::High => "HIGH",
            Confidence::Medium => "MEDIUM",
            Confidence::Low => "LOW",
        }
    }
}

/// Brightness measurement of one frame (or ROI).
#[derive(Debug, Clone)]
pub struct Brightness {
    pub min: u8,
    pub max: u8,
    pub mean: u8,
    pub lit: bool,
    pub threshold_used: u8,
    /// Fraction of ROI pixels ABOVE the threshold. Verdicts use the fraction,
    /// not the mean — a small bright LED must not be averaged away.
    pub lit_fraction: f32,
    pub bimodal: bool,
    pub confidence: Confidence,
    pub note: String,
}

/// A lit verdict implies a bright region; report its bbox as a SUGGESTED ROI
/// so the user can pin it for repeat runs (kills the coordinate-guessing
/// cold start).
fn suggest_roi(frame: &RgbaImage, luma_at: impl Fn(u32, u32) -> u8) -> Option<Rect> {
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return None;
    }
    // A 256-bucket histogram answers every percentile in O(n): cloning and
    // sorting all of a 1080p frame to read two values is wasted work when
    // luma has only 256 possible values.
    let mut hist = [0u64; 256];
    for y in 0..h {
        for x in 0..w {
            hist[luma_at(x, y) as usize] += 1;
        }
    }
    let total = (w as u64) * (h as u64);
    let percentile = |permille: u64| -> u8 {
        let target = (total - 1) * permille / 1000;
        let mut acc = 0u64;
        for (v, count) in hist.iter().enumerate() {
            acc += *count;
            if acc > target {
                return v as u8;
            }
        }
        255
    };
    // Background = the median. p99.5 alone misses a board LED entirely: at
    // 0.005% of the frame it never reaches the 99.5th percentile, which
    // still lands on the background — the suggestion would then be driven by
    // ambient light that no one asked about.
    let median = percentile(500);
    let p995 = percentile(995);
    // Relative margin above that background, never an absolute floor: a
    // dim-but-clear LED must be found just as well as a blazing one, while a
    // uniformly bright frame yields no suggestion at all (nothing stands
    // out, so there is no region worth proposing).
    let cutoff = median.saturating_add(60).max(p995);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut count = 0u64;
    let mut best = (0u32, 0u32, 0u8);
    for y in 0..h {
        for x in 0..w {
            let l = luma_at(x, y);
            if l >= cutoff {
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                if l > best.2 {
                    best = (x, y, l);
                }
            }
        }
    }
    if count == 0 {
        return None;
    }
    let (bw, bh) = (max_x - min_x + 1, max_y - min_y + 1);
    // Tiny candidate = a hot pixel/noise cluster: fall back to a small
    // window around the brightest pixel instead of a 1x1 box.
    if bw.saturating_mul(bh) <= 4 {
        let (cx, cy, _) = best;
        return Some(Rect {
            x: cx.saturating_sub(4),
            y: cy.saturating_sub(4),
            w: 9,
            h: 9,
        });
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        w: bw,
        h: bh,
    })
}

fn luma(px: &image::Rgba<u8>) -> u8 {
    let (r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32);
    ((r * 299 + g * 587 + b * 114) / 1000) as u8
}

/// Brightness analysis over the frame (or the Spec's ROI). Returns the
/// measurement plus a suggested ROI for repeat runs (lit frames only).
pub(crate) fn analyze_brightness(frame: &RgbaImage, spec: &Spec) -> (Brightness, Option<Rect>) {
    let (rx, ry, rw, rh) =
        spec.roi
            .map(|r| (r.x, r.y, r.w, r.h))
            .unwrap_or((0, 0, frame.width(), frame.height()));
    let mut lumas: Vec<u8> = Vec::with_capacity((rw.saturating_mul(rh)) as usize);
    for y in ry..(ry + rh).min(frame.height()) {
        for x in rx..(rx + rw).min(frame.width()) {
            lumas.push(luma(frame.get_pixel(x, y)));
        }
    }
    if lumas.is_empty() {
        return (
            Brightness {
                min: 0,
                max: 0,
                mean: 0,
                lit: false,
                threshold_used: 0,
                lit_fraction: 0.0,
                bimodal: false,
                confidence: Confidence::Low,
                note: "empty measurement region".to_string(),
            },
            None,
        );
    }
    let min = *lumas.iter().min().unwrap_or(&0);
    let max = *lumas.iter().max().unwrap_or(&0);
    let mean = (lumas.iter().map(|l| *l as u64).sum::<u64>() / lumas.len() as u64) as u8;
    let gap = max.saturating_sub(min);
    // Auto threshold: with a real light/dark split the min/max midpoint keys
    // on the frame itself; a UNIFORM frame has no split and the midpoint
    // degenerates to the luma itself (nothing would count as "above"), so
    // fall back to the absolute mid-scale.
    let auto_threshold = if gap >= 64 {
        min.saturating_add(max.saturating_sub(min) / 2)
    } else {
        128
    };
    let threshold = spec.threshold.unwrap_or(auto_threshold);
    let above = lumas.iter().filter(|l| **l > threshold).count();
    let lit_fraction = above as f32 / lumas.len() as f32;
    // Two-cluster separation: something bright AND something dark, both
    // actually present. NO fraction floor here — a board LED is a few pixels
    // and would be excluded by any percentage gate.
    let separated = gap >= 64 && above >= 1 && lumas.len() > above;
    // Balanced split (for the high-confidence verdict).
    let bimodal = separated && lit_fraction >= 0.01 && (1.0 - lit_fraction) >= 0.01;
    // Lit verdict: >= 2% of the ROI above the cutoff, OR any genuinely
    // bright pixel against a dim background (a board LED is a few pixels —
    // 4px on 320x240 is 0.005%, far below any sane fraction floor).
    //
    // The margin is RELATIVE to the threshold, not an absolute floor: a LED
    // behind a diffuser, shot off-angle or under-exposed can sit well below
    // 200 while being far brighter than its surroundings. An absolute cutoff
    // silently reported those as unlit.
    let tiny_bright = separated && max.saturating_sub(threshold) >= 40;
    let lit = lit_fraction >= 0.02 || tiny_bright;
    let (confidence, note) = if bimodal && (0.02..0.5).contains(&lit_fraction) {
        (
            Confidence::High,
            format!(
                "clean bimodal split, lit area {:.0}% of ROI",
                lit_fraction * 100.0
            ),
        )
    } else if separated && tiny_bright {
        (
            Confidence::Medium,
            format!(
                "small bright source ({:.3}% of ROI) against a dim background",
                lit_fraction * 100.0
            ),
        )
    } else if bimodal {
        (
            Confidence::Medium,
            format!(
                "bimodal, but lit area is {:.0}% of ROI",
                lit_fraction * 100.0
            ),
        )
    } else if lit {
        (
            Confidence::Medium,
            "entire ROI above the cutoff — uniformly bright".to_string(),
        )
    } else {
        (
            Confidence::Low,
            "no clear light/dark separation — ambient light may dominate".to_string(),
        )
    };

    // Suggested ROI only means something on a lit frame.
    let suggested = if lit {
        suggest_roi(frame, |x, y| luma(frame.get_pixel(x, y)))
    } else {
        None
    };

    (
        Brightness {
            min,
            max,
            mean,
            lit,
            threshold_used: threshold,
            lit_fraction,
            bimodal,
            confidence,
            note,
        },
        suggested,
    )
}

/// Mean luma over the frame (or the ROI) as a float. Used to spot a
/// whole-frame brightness shift, which usually means the camera changed
/// exposure rather than anything moving.
fn mean_luma(frame: &RgbaImage, roi: Option<Rect>) -> f32 {
    let (rx, ry, rw, rh) =
        roi.map(|r| (r.x, r.y, r.w, r.h))
            .unwrap_or((0, 0, frame.width(), frame.height()));
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in ry..(ry + rh).min(frame.height()) {
        for x in rx..(rx + rw).min(frame.width()) {
            sum += luma(frame.get_pixel(x, y)) as u64;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum as f32 / n as f32 }
}

/// Per-pixel comparison of two frames, restricted to `roi`.
///
/// Returns (fraction of pixels whose luma moved by at least
/// `pixel_threshold`, mean absolute luma difference, bounding box of the
/// pixels that moved). Shared by motion and diff.
pub(crate) fn frame_diff_map(
    a: &RgbaImage,
    b: &RgbaImage,
    roi: Option<Rect>,
    pixel_threshold: u8,
) -> Result<(f32, f32, Option<Rect>), String> {
    if a.dimensions() != b.dimensions() {
        return Err(format!(
            "frames differ in size: {}x{} vs {}x{}",
            a.width(),
            a.height(),
            b.width(),
            b.height()
        ));
    }
    let (rx, ry, rw, rh) =
        roi.map(|r| (r.x, r.y, r.w, r.h))
            .unwrap_or((0, 0, a.width(), a.height()));
    let mut changed = 0u64;
    let mut total = 0u64;
    let mut sum = 0u64;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in ry..(ry + rh).min(a.height()) {
        for x in rx..(rx + rw).min(a.width()) {
            let d = luma(a.get_pixel(x, y)).abs_diff(luma(b.get_pixel(x, y)));
            total += 1;
            sum += d as u64;
            if d >= pixel_threshold {
                changed += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if total == 0 {
        return Ok((0.0, 0.0, None));
    }
    // `then`, not `then_some`: the latter evaluates its argument eagerly, so
    // an unchanged pair (min_x still u32::MAX) would underflow in the
    // subtraction before the closure ever got a say.
    let bbox = (changed > 0).then(|| Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x + 1,
        h: max_y - min_y + 1,
    });
    Ok((
        changed as f32 / total as f32,
        sum as f32 / total as f32,
        bbox,
    ))
}

/// Per-pixel luma change that counts as "this pixel moved".
pub(crate) const MOTION_PIXEL_THRESHOLD: u8 = 16;

/// Motion verdict over a frame sequence.
#[derive(Debug)]
pub struct Motion {
    pub frames: usize,
    pub moving: bool,
    /// Strongest neighbouring-pair change, as a fraction of ROI pixels.
    pub changed_fraction: f32,
    pub mean_abs_diff: f32,
    /// Where the change happened (strongest pair).
    pub bbox: Option<Rect>,
    /// Whole-frame luma drift across the strongest pair. A large value means
    /// the camera changed exposure — not that something moved.
    pub global_shift: f32,
    pub confidence: Confidence,
    pub note: String,
}

pub(crate) fn analyze_motion(
    frames: &[RgbaImage],
    roi: Option<Rect>,
    pixel_threshold: u8,
) -> Result<Motion, String> {
    if frames.len() < 2 {
        return Err("motion needs at least 2 frames".to_string());
    }
    let mut changed_fraction = 0f32;
    let mut mean_abs_diff = 0f32;
    let mut bbox = None;
    let mut global_shift = 0f32;
    for pair in frames.windows(2) {
        let (frac, mean, box_here) = frame_diff_map(&pair[0], &pair[1], roi, pixel_threshold)?;
        let shift = (mean_luma(&pair[0], roi) - mean_luma(&pair[1], roi)).abs();
        if frac > changed_fraction {
            changed_fraction = frac;
            bbox = box_here;
        }
        mean_abs_diff = mean_abs_diff.max(mean);
        global_shift = global_shift.max(shift);
    }
    // Two gates, not one: the fraction alone is fooled by a single hot
    // pixel, and the mean alone is fooled by a uniform brightness drift.
    let moving = changed_fraction >= 0.01 && mean_abs_diff >= 2.0;
    // Did the whole frame get brighter or darker? Then it is exposure or
    // ambient light, and the motion verdict cannot be trusted.
    let exposure_suspect = mean_abs_diff > 0.0 && global_shift >= 0.5 * mean_abs_diff;
    let note = if !moving {
        format!(
            "nothing moved: {:.2}% of pixels changed by {pixel_threshold}+ luma \
             (mean diff {mean_abs_diff:.1})",
            changed_fraction * 100.0
        )
    } else if exposure_suspect {
        format!(
            "{:.1}% changed, but whole-frame luma also drifted {global_shift:.1} — likely \
             exposure or ambient light rather than motion",
            changed_fraction * 100.0
        )
    } else {
        format!(
            "{:.1}% of pixels changed (mean diff {mean_abs_diff:.1}) across {} frames",
            changed_fraction * 100.0,
            frames.len()
        )
    };
    let confidence = if !moving {
        // "Nothing moved" is a positive, safe finding.
        Confidence::High
    } else if exposure_suspect {
        Confidence::Low
    } else if changed_fraction >= 0.02 {
        Confidence::High
    } else {
        Confidence::Medium
    };
    Ok(Motion {
        frames: frames.len(),
        moving,
        changed_fraction,
        mean_abs_diff,
        bbox,
        global_shift,
        confidence,
        note,
    })
}

pub(crate) fn parse_roi(args: &Value, frame: &RgbaImage) -> Result<Option<Rect>, ToolError> {
    let Some(roi) = args.get("roi") else {
        return Ok(None);
    };
    let Some(items) = roi.as_array() else {
        return Err(ToolError::new(
            "[InvalidInput] roi must be an array [x, y, width, height]",
        ));
    };
    if items.len() != 4 {
        return Err(ToolError::new(
            "[InvalidInput] roi must have exactly 4 items: [x, y, width, height]",
        ));
    }
    let mut nums = [0u32; 4];
    for (i, v) in items.iter().enumerate() {
        let Some(n) = v.as_u64() else {
            return Err(ToolError::new(
                "[InvalidInput] roi items must be non-negative integers",
            ));
        };
        nums[i] =
            u32::try_from(n).map_err(|_| ToolError::new("[InvalidInput] roi item out of range"))?;
    }
    let (x, y, w, h) = (nums[0], nums[1], nums[2], nums[3]);
    if w == 0 || h == 0 {
        return Err(ToolError::new(
            "[InvalidInput] roi width/height must be >= 1",
        ));
    }
    if x.saturating_add(w) > frame.width() || y.saturating_add(h) > frame.height() {
        return Err(ToolError::new(format!(
            "[InvalidInput] roi [{x},{y},{w},{h}] exceeds the frame ({}x{})",
            frame.width(),
            frame.height()
        )));
    }
    Ok(Some(Rect { x, y, w, h }))
}

const NOT_IMPLEMENTED: &str = "arrives in v0.7.x — phase 1 supports mode=brightness only";

#[async_trait]
impl Tool for Observe {
    fn name(&self) -> &'static str {
        "observe"
    }

    fn description(&self) -> &'static str {
        "Analyze a photo of the target to verify PHYSICAL behavior (verification ladder rung 5): is the LED lit, how bright, where is the bright region. Deterministic local image analysis with a confidence rating — point the roi at the LED/indicator, not the whole board. Read-only."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["brightness", "blink", "motion", "diff"],
                    "description": "brightness: is the target lit, and how bright. motion: did anything move across a frame sequence. blink/diff arrive in a later release."
                },
                "path": {
                    "type": "string",
                    "description": "Image path inside the workspace (PNG/JPEG). Required for mode=brightness."
                },
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Frame sequence inside the workspace, in capture order — for mode=motion. A phone's burst mode works: copy the shots in order. Needs at least 2 frames, all the same size."
                },
                "pixel_threshold": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255,
                    "description": "motion: per-pixel luma change that counts as 'this pixel moved' (default 16). Raise it on grainy shots."
                },
                "roi": {
                    "type": "array",
                    "items": {"type": "integer", "minimum": 0},
                    "minItems": 4,
                    "maxItems": 4,
                    "description": "[x, y, width, height] region to measure; omit to measure the whole frame. Point it at the LED/indicator, not the whole board — a small lit area is easily averaged away."
                },
                "threshold": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255,
                    "description": "Light/dark cutoff. Omit to derive it from the frame's own brightness range."
                },
                "save": {
                    "type": "boolean",
                    "default": false,
                    "description": "Copy the measured image to .firment/observe/ for later review."
                }
            },
            "required": ["mode"]
        })
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        // Read-only analysis; even saving copies inside the workspace.
        None
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let mode = args
            .get("mode")
            .and_then(|m| m.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'mode'"))?;
        if !matches!(mode, "brightness" | "motion") {
            return Err(ToolError::new(format!(
                "[InvalidInput] mode={mode} is not implemented yet — {NOT_IMPLEMENTED}"
            )));
        }
        if mode == "motion" {
            return run_motion(&args, ctx);
        }
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] mode=brightness requires 'path'"))?;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        let frame = image::open(&resolved)
            .map_err(|e| ToolError::new(format!("[Io] cannot decode {}: {e}", resolved.display())))?
            .to_rgba8();

        let roi = parse_roi(&args, &frame)?;
        let threshold = match args.get("threshold").and_then(|t| t.as_u64()) {
            Some(t) if t > 255 => {
                return Err(ToolError::new("[InvalidInput] threshold must be 0..=255"));
            }
            Some(t) => Some(t as u8),
            None => None,
        };
        let mut source = StillSource { frame };
        let frame = source
            .next_frame()
            .map_err(ToolError::new)?
            .ok_or_else(|| ToolError::new("[Io] frame source produced no image"))?;
        let (b, suggested) = analyze_brightness(&frame, &Spec { roi, threshold });

        let mut text = format!(
            "[observe] mode=brightness roi=[{},{}]\n  luma: min={} max={} mean={}\n  lit: {} ({:.0}% of ROI above threshold {})\n  distribution: {}\n  confidence: {} — {}\n  frame: {}x{}, source=path {}",
            roi.map(|r| r.x.to_string()).unwrap_or_else(|| "-".into()),
            roi.map(|r| r.y.to_string()).unwrap_or_else(|| "-".into()),
            b.min,
            b.max,
            b.mean,
            if b.lit { "yes" } else { "no" },
            b.lit_fraction * 100.0,
            b.threshold_used,
            if b.bimodal { "bimodal" } else { "unimodal" },
            b.confidence.name(),
            b.note,
            frame.width(),
            frame.height(),
            path,
        );
        if let Some(roi) = suggested {
            text.push_str(&format!(
                "\n  suggested roi: [{}, {}, {}, {}] — the bright region; pass this back to measure it directly",
                roi.x, roi.y, roi.w, roi.h
            ));
        }

        if args.get("save").and_then(|s| s.as_bool()).unwrap_or(false) {
            let dir = ctx.cwd.join(".firment").join("observe");
            std::fs::create_dir_all(&dir)
                .map_err(|e| ToolError::new(format!("[Io] create {}: {e}", dir.display())))?;
            let file_name = resolved
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "frame.png".to_string());
            let dest = dir.join(format!(
                "{}-{file_name}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            std::fs::copy(&resolved, &dest)
                .map_err(|e| ToolError::new(format!("[Io] save {}: {e}", dest.display())))?;
            text.push_str(&format!("\n  saved: {}", dest.display()));
        }

        Ok(ToolOutput {
            text: truncate(&text, 32_000),
        })
    }
}

/// Appended to every sequence-mode verdict: these measurements compare
/// pixels, so they cannot tell a light turning on from the camera
/// changing its mind about exposure.
const CAMERA_CAVEAT: &str = "  note: pixel comparison cannot tell a light switching on from exposure or \
     ambient change — fix the camera settings (or set roi) before trusting a \
     marginal verdict.";

/// mode=motion: read a frame sequence in capture order and report whether
/// anything moved.
fn run_motion(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
    let paths = args
        .get("paths")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ToolError::new(
                "[InvalidInput] mode=motion requires 'paths' (2+ frames, in capture order)",
            )
        })?;
    if paths.len() < 2 {
        return Err(ToolError::new(
            "[InvalidInput] mode=motion requires at least 2 frames in 'paths'",
        ));
    }
    let pixel_threshold = match args.get("pixel_threshold").and_then(|v| v.as_u64()) {
        Some(t) if t > 255 => {
            return Err(ToolError::new(
                "[InvalidInput] pixel_threshold must be 0..=255",
            ));
        }
        Some(t) => t as u8,
        None => MOTION_PIXEL_THRESHOLD,
    };

    let mut frames = Vec::with_capacity(paths.len());
    for (i, entry) in paths.iter().enumerate() {
        let Some(p) = entry.as_str() else {
            return Err(ToolError::new(format!(
                "[InvalidInput] paths[{i}] must be a string"
            )));
        };
        let resolved = resolve_within(&ctx.cwd, p, &ctx.allowed_roots)
            .map_err(|e| ToolError::new(format!("[Permission] frames[{i}] ({p}): {e}")))?;
        let img = image::open(&resolved)
            .map_err(|e| {
                ToolError::new(format!(
                    "[Io] cannot decode frames[{i}] ({}): {e}",
                    resolved.display()
                ))
            })?
            .to_rgba8();
        frames.push(img);
    }

    // ROI bounds are validated against the first frame; equal sizes across
    // the whole sequence are checked inside the analysis.
    let roi = parse_roi(args, &frames[0])?;
    let m = analyze_motion(&frames, roi, pixel_threshold).map_err(ToolError::new)?;

    let mut text = format!(
        "[observe] mode=motion frames={} roi=[{},{}]\n  moving: {}\n  changed: {:.1}% of ROI \
         (pixel threshold {})\n  mean abs diff: {:.1}\n  exposure drift: {:.1}\n  \
         confidence: {} — {}\n",
        m.frames,
        roi.map(|r| r.x.to_string()).unwrap_or_else(|| "-".into()),
        roi.map(|r| r.y.to_string()).unwrap_or_else(|| "-".into()),
        if m.moving { "yes" } else { "no" },
        m.changed_fraction * 100.0,
        pixel_threshold,
        m.mean_abs_diff,
        m.global_shift,
        m.confidence.name(),
        m.note,
    );
    if let Some(b) = m.bbox {
        text.push_str(&format!(
            "  bbox: [{}, {}, {}, {}] — where the change is\n",
            b.x, b.y, b.w, b.h
        ));
    }
    text.push_str(&format!(
        "  frame: {}x{}, source=paths ({} files)\n",
        frames[0].width(),
        frames[0].height(),
        paths.len()
    ));
    text.push_str(CAMERA_CAVEAT);

    Ok(ToolOutput {
        text: truncate(&text, 32_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    fn solid(w: u32, h: u32, l: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([l, l, l, 255]))
    }

    /// Black frame with a bright square at (x, y).
    fn frame_with_block(x: u32, y: u32, size: u32, l: u8) -> RgbaImage {
        let mut f = solid(64, 64, 8);
        for dy in 0..size {
            for dx in 0..size {
                f.put_pixel(x + dx, y + dy, image::Rgba([l, l, l, 255]));
            }
        }
        f
    }

    #[test]
    fn motion_detects_moving_block() {
        // The block moves from (10,10) to (30,30): both the old and the new
        // position should be inside the reported bounding box.
        let frames = vec![
            frame_with_block(10, 10, 8, 240),
            frame_with_block(30, 30, 8, 240),
        ];
        let m = analyze_motion(&frames, None, MOTION_PIXEL_THRESHOLD).unwrap();
        assert!(m.moving, "a moving block is motion: {}", m.note);
        let bbox = m.bbox.expect("motion reports where it happened");
        assert!(
            bbox.x <= 10 && bbox.y <= 10,
            "covers the old spot: {bbox:?}"
        );
        assert!(
            bbox.x + bbox.w >= 38 && bbox.y + bbox.h >= 38,
            "covers the new spot: {bbox:?}"
        );
    }

    #[test]
    fn motion_static_frames_report_no_motion() {
        let frames = vec![
            frame_with_block(10, 10, 8, 240),
            frame_with_block(10, 10, 8, 240),
        ];
        let m = analyze_motion(&frames, None, MOTION_PIXEL_THRESHOLD).unwrap();
        assert!(!m.moving, "identical frames are not motion: {}", m.note);
        assert_eq!(m.confidence.name(), "HIGH");
    }

    #[test]
    fn motion_ignores_a_single_hot_pixel() {
        // One flickering pixel is sensor noise, not a motor turning: the
        // fraction gate must reject it even though the delta is large.
        let mut a = solid(64, 64, 8);
        let mut b = a.clone();
        a.put_pixel(5, 5, image::Rgba([255, 255, 255, 255]));
        b.put_pixel(5, 5, image::Rgba([0, 0, 0, 255]));
        let m = analyze_motion(&[a, b], None, MOTION_PIXEL_THRESHOLD).unwrap();
        assert!(
            !m.moving,
            "a single pixel is noise, not motion: frac={}",
            m.changed_fraction
        );
    }

    #[test]
    fn motion_flags_exposure_shift_as_low_confidence() {
        // A block moves AND the whole frame brightens: the verdict may be
        // right, but the evidence is contaminated, so it must say so.
        let mut frames = vec![
            frame_with_block(10, 10, 8, 240),
            frame_with_block(30, 30, 8, 240),
        ];
        for px in frames[1].pixels_mut() {
            px[0] = px[0].saturating_add(60);
            px[1] = px[1].saturating_add(60);
            px[2] = px[2].saturating_add(60);
        }
        let m = analyze_motion(&frames, None, MOTION_PIXEL_THRESHOLD).unwrap();
        assert!(m.moving, "a block did move: {}", m.note);
        assert_eq!(
            m.confidence.name(),
            "LOW",
            "whole-frame drift must downgrade the verdict: {}",
            m.note
        );
    }

    #[test]
    fn motion_needs_two_frames() {
        let err = analyze_motion(&[solid(16, 16, 8)], None, MOTION_PIXEL_THRESHOLD).unwrap_err();
        assert!(err.contains("at least 2 frames"), "got: {err}");
    }

    #[test]
    fn frame_diff_map_rejects_mismatched_sizes() {
        let err = frame_diff_map(&solid(16, 16, 8), &solid(32, 32, 8), None, 16).unwrap_err();
        assert!(err.contains("differ in size"), "got: {err}");
    }

    #[test]
    fn brightness_detects_lit_region() {
        // Black frame with a white 40x40 block at (120, 80) of 320x240.
        let mut frame = solid(320, 240, 10);
        for y in 80..120 {
            for x in 120..160 {
                frame.put_pixel(x, y, image::Rgba([250, 250, 250, 255]));
            }
        }
        let (b, _) = analyze_brightness(
            &frame,
            &Spec {
                roi: Some(Rect {
                    x: 120,
                    y: 80,
                    w: 40,
                    h: 40,
                }),
                threshold: None,
            },
        );
        assert!(b.lit);
        assert!(b.lit_fraction > 0.95, "solid white roi: {}", b.lit_fraction);
        // Uniform bright: confidently lit, but there is no light/dark edge
        // to key the auto-threshold on.
        assert_eq!(b.confidence, Confidence::Medium);
    }

    #[test]
    fn brightness_detects_dark_frame() {
        let frame = solid(320, 240, 5);
        let (b, suggested) = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: None,
            },
        );
        assert!(!b.lit);
        assert!(
            suggested.is_none(),
            "a dark frame has no bright region to suggest"
        );
    }

    #[test]
    fn roi_limits_measurement() {
        // Left half bright, right half dark.
        let mut frame = solid(200, 100, 10);
        for y in 0..100 {
            for x in 0..100 {
                frame.put_pixel(x, y, image::Rgba([240, 240, 240, 255]));
            }
        }
        let left = analyze_brightness(
            &frame,
            &Spec {
                roi: Some(Rect {
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 100,
                }),
                threshold: None,
            },
        )
        .0;
        let right = analyze_brightness(
            &frame,
            &Spec {
                roi: Some(Rect {
                    x: 100,
                    y: 0,
                    w: 100,
                    h: 100,
                }),
                threshold: None,
            },
        )
        .0;
        assert!(
            left.mean > right.mean,
            "left {} right {}",
            left.mean,
            right.mean
        );
        assert!(left.lit && !right.lit);
    }

    #[test]
    fn small_lit_area_is_not_averaged_away() {
        // A 4px bright dot on black: mean-based judgment would call this
        // dark; the lit_fraction verdict must catch it.
        let mut frame = solid(320, 240, 8);
        for (x, y) in [(100, 100), (101, 100), (100, 101), (101, 101)] {
            frame.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
        let (b, suggested) = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: None,
            },
        );
        assert!(b.lit, "4px LED must register: frac={}", b.lit_fraction);
        // The auto-ROI suggestion points at the dot and is small.
        let r = suggested.expect("lit frame suggests an roi");
        assert!(r.w * r.h >= 1 && r.x <= 101 && r.y <= 101);
    }

    #[test]
    fn dim_led_still_registers_as_lit() {
        // Regression: the verdict used to demand an absolute luma >= 200, so
        // an LED behind a diffuser, shot off-angle or under-exposed read as
        // UNLIT even at ~19x the background. Brightness is relative — what
        // matters is the margin over the surroundings.
        let mut frame = solid(320, 240, 8);
        for (x, y) in [(100, 100), (101, 100), (100, 101), (101, 101)] {
            frame.put_pixel(x, y, image::Rgba([150, 150, 150, 255]));
        }
        let (b, suggested) = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: None,
            },
        );
        assert!(
            b.lit,
            "a dim-but-clear LED must register: frac={}",
            b.lit_fraction
        );
        // It must also still be locatable, not swallowed by the background.
        let r = suggested.expect("lit frame suggests an roi");
        assert!(
            r.x <= 101 && r.y <= 101 && r.x + r.w >= 101 && r.y + r.h >= 101,
            "suggestion must cover the LED: {r:?}"
        );
    }

    #[test]
    fn single_hot_pixel_is_not_trusted_as_the_suggestion() {
        // One 255 pixel in a mid-gray frame: p99.5 lands on the 255s, the
        // candidate set is tiny, and the fallback window must not become a
        // 1x1 box pinned to noise.
        let mut frame = solid(320, 240, 160);
        frame.put_pixel(50, 50, image::Rgba([255, 255, 255, 255]));
        let (b, suggested) = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: None,
            },
        );
        assert!(b.lit);
        let r = suggested.expect("lit frame suggests an roi");
        assert!(r.w >= 8 && r.h >= 8, "fallback window: {:?}", r);
    }

    #[test]
    fn threshold_override_flips_verdict() {
        let frame = solid(64, 64, 100); // uniform mid-gray
        let dark = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: Some(120),
            },
        )
        .0;
        let lit = analyze_brightness(
            &frame,
            &Spec {
                roi: None,
                threshold: Some(60),
            },
        )
        .0;
        // Uniform frame: nothing is ABOVE the cutoff when it equals the luma,
        // everything is when the cutoff sits below it.
        assert!(!dark.lit, "luma 100 is not > 120");
        assert!(lit.lit, "luma 100 is > 60");
    }

    #[tokio::test]
    async fn path_outside_workspace_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("outside.png");
        let err = Observe
            .run(
                json!({"mode": "brightness", "path": outside.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[Permission]"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn unsupported_mode_states_not_implemented() {
        let dir = tempdir().unwrap();
        let err = Observe
            .run(json!({"mode": "blink"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("not implemented"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn roi_out_of_bounds_is_rejected() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.png"), b"not a real png").ok();
        // Use a real image so decoding succeeds before roi validation.
        let frame = solid(64, 64, 100);
        frame.save(dir.path().join("real.png")).unwrap();
        let err = Observe
            .run(
                json!({"mode": "brightness", "path": "real.png", "roi": [60, 60, 10, 10]}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[InvalidInput] roi"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn registered_in_all() {
        assert!(crate::tools::all().iter().any(|t| t.name() == "observe"));
    }

    #[test]
    fn plan_registry_includes_observe() {
        let reg = crate::plan_registry();
        assert!(reg.get("observe").is_some());
    }
}
