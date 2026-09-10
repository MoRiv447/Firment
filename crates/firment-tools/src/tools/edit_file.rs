use super::util::{read_text, resolve_within, simple_diff};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Edit a file by exact old_text anchor (must match exactly once), by 1-based line range, or \
         as a batch of edits in one call."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string", "description": "Exact text to replace; must occur exactly once. If it does not match, the error names the closest location — copy that text verbatim rather than retrying blind."},
                "expected_sha256": {"type": "string", "description": "Optional SHA-256 of the file content as read (from read_file footer); mismatches abort with [ConcurrentChange]"},
                "start_line": {"type": "integer", "minimum": 1},
                "end_line": {"type": "integer", "minimum": 1},
                "new_text": {"type": "string", "description": "Replacement text"},
                "hashline": {"type": "string", "description": "8-hex content hash anchor of the first line to replace (from read_file hashlines=true); the file must still contain exactly one line with this hash"},
                "end_hashline": {"type": "string", "description": "8-hex content hash of the last line of the range (optional, same mode as hashline)"},
                "ignore_trailing_whitespace": {"type": "boolean", "description": "Match old_text ignoring differences in TRAILING whitespace only — leading indentation is never ignored. Defaults to true, except for Python/Makefile/CMake/YAML where trailing whitespace can be significant."},
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Batch mode: several edits to one file in a single call, applied IN ORDER, each seeing the previous result. Prefer this over repeated calls when a change spans several hunks. All or nothing: the first failure aborts the whole call and the file is untouched.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": {"type": "string"},
                            "start_line": {"type": "integer", "minimum": 1},
                            "end_line": {"type": "integer", "minimum": 1},
                            "hashline": {"type": "string"},
                            "end_hashline": {"type": "string"},
                            "new_text": {"type": "string"}
                        },
                        "required": ["new_text"],
                        "oneOf": [
                            {"required": ["old_text"]},
                            {"required": ["start_line"]},
                            {"required": ["hashline"]}
                        ]
                    }
                }
            },
            "required": ["path"],
            "oneOf": [
                {"required": ["new_text", "old_text"]},
                {"required": ["new_text", "start_line"]},
                {"required": ["new_text", "hashline"]},
                {"required": ["edits"]}
            ]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        args.get("path")
            .and_then(|p| p.as_str())
            .map(|p| format!("edit file {p}"))
    }

    fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<String> {
        let path = args.get("path")?.as_str()?;
        let resolved = resolve_within(&ctx.cwd, path, &ctx.allowed_roots).ok()?;
        let original = read_text(&resolved).ok()?;
        let new_content = compute_edit(&resolved, &original, args).ok()?;
        Some(simple_diff(&resolved, &original, &new_content, 4000))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'path'"))?;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        let original = read_text(&resolved).map_err(|e| {
            if resolved.exists() {
                ToolError::new(format!("[Io] {e}"))
            } else {
                ToolError::new(format!(
                    "[NotFound] {e} — the file does not exist yet; create it with write_file \
                     first, or fix the path"
                ))
            }
        })?;
        let original_bytes = fs::read(&resolved)
            .map_err(|e| ToolError::new(format!("[Io] cannot read {}: {e}", resolved.display())))?;
        // read_text above decodes lossily, so editing a non-UTF-8 file (GBK,
        // Latin-1, ...) would rewrite EVERY invalid byte as U+FFFD on save —
        // whole-file corruption far beyond the edited hunk. Refuse instead.
        if std::str::from_utf8(&original_bytes).is_err() {
            return Err(ToolError::new(format!(
                "[Encoding] {} is not valid UTF-8 — refusing to edit: the text pipeline \
                 would permanently replace every non-UTF-8 byte with U+FFFD across the \
                 whole file. Convert it to UTF-8 first.",
                resolved.display()
            )));
        }
        if let Some(expected) = args.get("expected_sha256").and_then(|e| e.as_str()) {
            let current = firment_core::hash::sha256_hex(&original_bytes);
            if current != expected {
                return Err(ToolError::new(format!(
                    "[ConcurrentChange] file hash mismatch (expected {expected}, current \
                     {current}): re-read the file with read_file and retry"
                )));
            }
        }
        let new_content = compute_edit(&resolved, &original, &args)?;
        if new_content == original {
            return Err(ToolError::new(
                "[InvalidInput] the edit produced no change (target content equals replacement \
                 content). The problem is likely elsewhere: re-read the file first; do not \
                 widen the anchor or resubmit the same edit.",
            ));
        }

        ctx.journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(&resolved)
            .map_err(ToolError::new)?;
        // CAS: refuse to apply the edit if the file changed since we read it.
        let current =
            fs::read(&resolved).map_err(|e| ToolError::new(format!("[Io] re-read failed: {e}")))?;
        if current != original_bytes {
            return Err(ToolError::new(format!(
                "[ConcurrentChange] file changed during edit (concurrent modification), aborted: {}",
                resolved.display()
            )));
        }
        fs::write(&resolved, &new_content)
            .map_err(|e| ToolError::new(format!("[Io] write failed: {e}")))?;
        let old_lines = original.lines().count();
        let new_lines = new_content.lines().count();
        // Echo the actual change as a unified diff so the model sees exactly
        // what landed and does not need to re-read the file to confirm.
        let diff = simple_diff(&resolved, &original, &new_content, 4000);
        Ok(ToolOutput {
            text: format!(
                "Edited {} ({} lines -> {} lines)\n{diff}",
                resolved.display(),
                old_lines,
                new_lines
            ),
        })
    }
}

fn compute_edit(resolved: &Path, original: &str, args: &Value) -> Result<String, ToolError> {
    let new_text = args.get("new_text").and_then(|n| n.as_str());
    let old_text = args.get("old_text").and_then(|o| o.as_str());
    let start_line = args.get("start_line").and_then(|s| s.as_u64());
    let hashline = args.get("hashline").and_then(|h| h.as_str());
    let edits = args.get("edits").and_then(|e| e.as_array());
    let ignore_trailing = tolerates_trailing_whitespace(resolved, args);

    let crlf = detect_crlf(original);
    let body = original.strip_prefix('\u{FEFF}');
    let has_bom = body.is_some();
    let body = body.unwrap_or(original);
    let content = body.replace("\r\n", "\n");

    let crlf_restore = |text: String| {
        if crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    };
    let restore = |text: String| {
        let text = crlf_restore(text);
        if has_bom {
            format!("\u{FEFF}{text}")
        } else {
            text
        }
    };

    if let Some(items) = edits {
        if old_text.is_some() || start_line.is_some() || hashline.is_some() || new_text.is_some() {
            return Err(ToolError::new(
                "[InvalidInput] use either the single-edit fields (old_text / start_line / \
                 hashline + new_text) or 'edits', not both",
            ));
        }
        if items.is_empty() {
            return Err(ToolError::new("[InvalidInput] 'edits' is empty"));
        }
        // Folded in order, each edit seeing the previous one's result. Sequential
        // is the only semantics that composes: every anchor is written against a
        // file the model has actually read, and offsets in one spec cannot be
        // interpreted against two different versions of the text.
        let mut current = content;
        for (i, item) in items.iter().enumerate() {
            let obj = item.as_object().ok_or_else(|| {
                ToolError::new(format!("[InvalidInput] {} is not an object", edit_label(i)))
            })?;
            let value = Value::Object(obj.clone());
            current = apply_edit_spec(resolved, &current, &value, edit_label(i), ignore_trailing)?;
        }
        return Ok(restore(current));
    }

    let single = apply_edit_spec(resolved, &content, args, String::new(), ignore_trailing)?;
    Ok(restore(single))
}

/// Apply one edit — either the top-level arguments or one item of `edits` — to
/// `content`, which is already LF-normalised and BOM-free.
///
/// `label` prefixes every error so a batch failure names the item instead of
/// leaving the model to guess which of six edits was wrong.
fn apply_edit_spec(
    resolved: &Path,
    content: &str,
    spec: &Value,
    label: String,
    ignore_trailing: bool,
) -> Result<String, ToolError> {
    let prefix = if label.is_empty() {
        String::new()
    } else {
        format!("{label}: ")
    };
    let tagged = |msg: String| ToolError::new(format!("[InvalidInput] {prefix}{msg}"));

    let new_text = spec
        .get("new_text")
        .and_then(|n| n.as_str())
        .ok_or_else(|| {
            ToolError::new(format!(
                "[InvalidInput] {prefix}missing 'new_text'{}",
                if label.is_empty() {
                    ""
                } else {
                    " in this item"
                }
            ))
        })?;
    let new_norm = new_text.replace("\r\n", "\n");
    let old_text = spec.get("old_text").and_then(|o| o.as_str());
    let hashline = spec.get("hashline").and_then(|h| h.as_str());

    if let Some(anchor) = hashline {
        let end_hashline = spec.get("end_hashline").and_then(|h| h.as_str());
        return edit_by_hashline(content, anchor, end_hashline, &new_norm)
            .map_err(|e| ToolError::new(format!("[InvalidInput] {prefix}{}", e.message)));
    }

    if let Some(old) = old_text {
        let old_norm = old.replace("\r\n", "\n");
        return match locate_old_text(content, &old_norm, ignore_trailing) {
            Ok((begin, end)) => {
                let mut out = String::with_capacity(content.len());
                out.push_str(&content[..begin]);
                out.push_str(&new_norm);
                out.push_str(&content[end..]);
                Ok(out)
            }
            Err(e) if e.message.contains("not found") => {
                // A 0-match failure is the one worth spending work on: the model
                // usually typed a near-correct anchor and one round trip is the
                // whole cost difference. Never applied automatically -- the
                // suggestion only says where it looked closest.
                let hint = fuzzy_suggestion(content, &old_norm, ignore_trailing, 200)
                    .unwrap_or_else(|| e.message.clone());
                // The label has to survive: in a batch of six edits, "closest
                // match is line 40" without saying WHICH item is unusable.
                Err(ToolError::new(format!("[InvalidInput] {prefix}{hint}")))
            }
            Err(e) => Err(ToolError::new(format!(
                "[InvalidInput] {prefix}{}",
                e.message
            ))),
        };
    }

    let start_line = spec.get("start_line").and_then(|s| s.as_u64());
    let end_line = spec.get("end_line").and_then(|e| e.as_u64());
    let start = start_line
        .ok_or_else(|| tagged("provide 'old_text', 'start_line' or 'hashline'".to_string()))?
        as usize;
    let end = end_line.unwrap_or(start as u64) as usize;
    if start == 0 || end < start {
        return Err(tagged("invalid line range".to_string()));
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    let trailing_newline = content.ends_with('\n');
    if trailing_newline {
        lines.pop();
    }
    if start > lines.len() || end > lines.len() {
        return Err(tagged(format!(
            "line range {start}..{end} out of bounds (file has {} lines)",
            lines.len()
        )));
    }
    let _ = resolved;
    let mut out: Vec<&str> = Vec::new();
    out.extend_from_slice(&lines[..start - 1]);
    out.extend(new_norm.split('\n'));
    out.extend_from_slice(&lines[end..]);
    let mut joined = out.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    Ok(joined)
}

/// A label for one edit inside a batch, used in error messages so the model
/// knows WHICH item failed rather than just that the call did.
fn edit_label(i: usize) -> String {
    format!("edits[{i}]")
}

/// The newline style to restore on save, decided by MAJORITY.
///
/// `contains("\r\n")` let one stray CRLF in a mostly-LF file rewrite every line
/// ending of the file.
fn detect_crlf(original: &str) -> bool {
    let lf = original.matches('\n').count();
    let crlf = original.matches("\r\n").count();
    crlf * 2 > lf
}

/// Files where trailing whitespace can be significant, so the tolerance is off
/// by default.
///
/// Python and YAML are indentation-sensitive by spec, and a Makefile recipe's
/// difference between spaces and a tab is the difference between a working and
/// a broken build. In these the model gets the exact behaviour it had before,
/// and can still opt in per call.
fn whitespace_significant(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "makefile" || name == "gnumakefile" || name.starts_with("makefile.") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("py" | "pyi" | "mk" | "cmake" | "yml" | "yaml")
    ) || name == "cmakelists.txt"
}

/// Whether a tolerant match is allowed: an explicit argument wins, otherwise
/// everything except the whitespace-significant file types.
fn tolerates_trailing_whitespace(path: &Path, args: &Value) -> bool {
    args.get("ignore_trailing_whitespace")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| !whitespace_significant(path))
}

/// Matching key: trailing whitespace removed, LEADING whitespace kept.
///
/// The asymmetry is the whole point. A model that reflows a line often drops
/// its trailing spaces, and that difference is invisible in a rendered diff —
/// it costs a whole round trip for nothing. Leading whitespace is the
/// indentation, which is syntax in Python and Makefile and is how a reader
/// tells nesting apart everywhere else, so a match that ignores it would edit
/// the wrong construct.
fn match_key(line: &str, ignore_trailing: bool) -> &str {
    if ignore_trailing {
        line.trim_end()
    } else {
        line
    }
}

/// The line ranges of `text` after normalising its newlines.
fn line_slice(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// Best-effort "did you mean this?" for an `old_text` that did not match.
///
/// Returns `None` when the search would be too expensive to be worth it, which
/// is deliberate: a 1 MB file is exactly where a silent multi-second stall
/// would be least welcome, and the plain "matched 0 times" message is still
/// correct there.
fn fuzzy_suggestion(
    content: &str,
    wanted: &str,
    ignore_trailing: bool,
    cap: usize,
) -> Option<String> {
    /// Ceiling on line_count * excerpt_length. At the cap the file is around
    /// 1 MB with a 2 KB excerpt, which measures in the low hundreds of
    /// milliseconds; past it the search is skipped.
    const MAX_WORK: usize = 20_000_000;

    let lines = line_slice(content);
    let want = line_slice(wanted);
    if want.is_empty() || lines.is_empty() {
        return None;
    }
    if lines.len().saturating_mul(wanted.len()) > MAX_WORK {
        return None;
    }

    let want_keys: Vec<&str> = want.iter().map(|l| match_key(l, ignore_trailing)).collect();
    let want_text = want_keys.join("\n");

    let mut best: Option<(usize, usize, usize)> = None; // (distance, start, end)
    for start in 0..lines.len() {
        let end = (start + want.len()).min(lines.len());
        let candidate = lines[start..end]
            .iter()
            .map(|l| match_key(l, ignore_trailing))
            .collect::<Vec<_>>()
            .join("\n");
        // Lines beyond the window would be an insertion, so score the window
        // against the search text only; the exact-delete shape is the common one.
        let d = levenshtein_capped(&want_text, &candidate, cap);
        if d >= cap {
            continue;
        }
        if best.is_none_or(|(best_d, _, _)| d < best_d) {
            best = Some((d, start, end));
        }
    }

    let (distance, start, end) = best?;
    let matched = lines[start..end].join("\n");
    Some(format!(
        "old_text not found in this file. Closest match is lines {}-{} ({} character(s) \
         different):\n---\n{matched}\n---\nCopy that text exactly, or re-read the file with \
         read_file and use a hashline anchor. Nothing was changed.",
        start + 1,
        end,
        distance,
    ))
}

/// Levenshtein distance, abandoning once it is certain to exceed `cap`.
///
/// The exact number does not matter -- only whether a candidate is close enough
/// to be worth showing -- so the cap turns a full O(n*m) pass into an early
/// exit on the many hopeless candidates a file full of code produces.
fn levenshtein_capped(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) >= cap {
        return cap;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if row_min >= cap {
            return cap;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Locate `old_text` in `content` and return the byte span to replace.
///
/// The anchor is matched as WHOLE LINES, in both the exact and the tolerant
/// pass, and the exact pass runs first so tolerance can never shadow a literal
/// match. Whole lines for two reasons:
///
/// * A substring search let the anchor match in the middle of a line, so
///   `"= 1"` matched inside `"x = 1"` and the "replace exactly this" contract
///   was quietly false.
/// * It made the trailing-whitespace tolerance unreachable for the very case it
///   exists for: `"x = 1"` was found inside `"x = 1  "` by the exact pass, so a
///   `.py` file or an `ignore_trailing_whitespace: false` call never got the
///   strict behaviour it had asked for.
fn locate_old_text(
    content: &str,
    old: &str,
    ignore_trailing: bool,
) -> Result<(usize, usize), ToolError> {
    let (hits, span) = find_line_windows(content, old, false);
    if hits == 1 {
        return Ok(span.expect("one hit carries its span"));
    }
    if hits > 1 {
        return Err(ambiguous(hits, false));
    }
    if !ignore_trailing {
        return Err(not_found());
    }
    let (hits, span) = find_line_windows(content, old, true);
    match hits {
        1 => Ok(span.expect("one hit carries its span")),
        0 => Err(not_found()),
        n => Err(ambiguous(n, true)),
    }
}

fn not_found() -> ToolError {
    ToolError::new("[InvalidInput] old_text not found in the file")
}

fn ambiguous(hits: usize, tolerant: bool) -> ToolError {
    let how = if tolerant {
        "once trailing whitespace is ignored"
    } else {
        ""
    };
    ToolError::new(format!(
        "[InvalidInput] old_text matched {hits} times{}{} — expected exactly 1: extend the anchor \
         with surrounding lines to make it unique",
        if tolerant { " " } else { "" },
        how,
    ))
}

/// Line-aligned occurrences of `old` in `content`, as (count, first span).
///
/// `count` may exceed 1 without the span being meaningful; the caller only
/// errors on that, and stops the scan early to keep a pathological anchor cheap.
fn find_line_windows(
    content: &str,
    old: &str,
    ignore_trailing: bool,
) -> (usize, Option<(usize, usize)>) {
    let want: Vec<&str> = old.lines().collect();
    if want.is_empty() {
        return (0, None);
    }
    let want_keys: Vec<&str> = want.iter().map(|l| match_key(l, ignore_trailing)).collect();

    let lines: Vec<&str> = content.lines().collect();
    // Byte offset of each line's start, so a matched LINE range maps back to
    // offsets in the real text and the replacement preserves everything else.
    let mut offsets = Vec::with_capacity(lines.len());
    let mut pos = 0usize;
    for line in &lines {
        offsets.push(pos);
        pos += line.len() + 1; // the '\n' that `lines()` elided
    }

    let mut count = 0usize;
    let mut first = None;
    for start in 0..lines.len() {
        if start + want.len() > lines.len() {
            break;
        }
        let matches = lines[start..start + want.len()]
            .iter()
            .zip(&want_keys)
            .all(|(line, key)| match_key(line, ignore_trailing) == *key);
        if matches {
            count += 1;
            if first.is_none() {
                let last = start + want.len() - 1;
                first = Some((offsets[start], offsets[last] + lines[last].len()));
            }
            if count > 1 {
                return (count, None);
            }
        }
    }
    (count, first)
}

/// omp-style hashline edit: locate lines by 8-hex content-hash anchors.
fn edit_by_hashline(
    original: &str,
    hashline: &str,
    end_hashline: Option<&str>,
    new_text: &str,
) -> Result<String, ToolError> {
    let mut lines: Vec<&str> = original.split('\n').collect();
    let trailing_newline = original.ends_with('\n');
    if trailing_newline {
        lines.pop();
    }
    let full_hash = |line: &str| firment_core::hash::sha256_hex(line.as_bytes());
    let find = |anchor: &str| -> Result<usize, ToolError> {
        // An empty (or all-whitespace) anchor would start_with-match every
        // line and "uniquely" resolve in a single-line file — an edit with no
        // anchor at all. The schema asks for the 8-hex hash; enforce it.
        if anchor.len() < 8 || !anchor.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ToolError::new(format!(
                "[InvalidInput] hashline anchor {anchor:?} is not a non-empty 8-hex (or longer) \
                 content hash; re-read with read_file hashlines=true and copy an anchor"
            )));
        }
        let anchor = anchor.to_lowercase();
        let matches: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| full_hash(line).starts_with(&anchor))
            .map(|(i, _)| i)
            .collect();
        match matches.len() {
            1 => Ok(matches[0]),
            0 => Err(ToolError::new(format!(
                "[ConcurrentChange] anchor hash {anchor} not found in the file: it may have \
                 changed; re-read with read_file and retry"
            ))),
            _ => Err(ToolError::new(format!(
                "[InvalidInput] anchor hash {anchor} matches {} line(s), not unique: re-read \
                 with read_file hashlines=true and use a longer hash",
                matches.len()
            ))),
        }
    };
    let start = find(hashline)?;
    let end = match end_hashline {
        Some(end) => {
            let end = find(end)?;
            if end < start {
                return Err(ToolError::new(
                    "[InvalidInput] end_hashline is before hashline; invalid range",
                ));
            }
            end
        }
        None => start,
    };
    let mut out: Vec<&str> = Vec::new();
    out.extend_from_slice(&lines[..start]);
    out.extend(new_text.split('\n'));
    out.extend_from_slice(&lines[end + 1..]);
    let mut joined = out.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    Ok(joined)
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
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn preview_shows_edit_diff() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        let preview = EditFile
            .preview(
                &json!({"path": "a.txt", "old_text": "hello", "new_text": "hi"}),
                &ctx(dir.path()),
            )
            .unwrap();
        assert!(preview.contains("-hello"), "got: {preview}");
        assert!(preview.contains("+hi"), "got: {preview}");
    }

    #[tokio::test]
    async fn crlf_file_matches_lf_anchor_and_keeps_crlf() {
        // CubeMX/Keil files are CRLF; the model writes LF anchors. The anchor
        // must match and the written file must keep CRLF endings (no mixed
        // endings, no LF-only diff).
        let dir = tempdir().unwrap();
        let path = dir.path().join("main.c");
        std::fs::write(
            &path,
            "/* USER CODE BEGIN PV */\r\nint x;\r\n/* USER CODE END PV */\r\n",
        )
        .unwrap();
        let out = EditFile
            .run(
                json!({
                    "path": "main.c",
                    "old_text": "/* USER CODE BEGIN PV */",
                    "new_text": "/* USER CODE BEGIN PV */\nint counter = 0;"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("Edited"), "got: {}", out.text);
        let content = std::fs::read(&path).unwrap();
        assert!(
            String::from_utf8_lossy(&content).contains("int counter = 0;"),
            "replacement must land"
        );
        let crlf = content.windows(2).filter(|w| w == b"\r\n").count();
        let lf = content.windows(1).filter(|w| w == b"\n").count();
        assert_eq!(
            crlf,
            lf,
            "every LF must be part of CRLF (no mixed endings): {:?}",
            String::from_utf8_lossy(&content)
        );
    }

    #[tokio::test]
    async fn lf_file_keeps_lf_line_endings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "aaa\nbbb\n").unwrap();
        let out = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "aaa", "new_text": "AAA\nAAA2"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("Edited"), "got: {}", out.text);
        let content = std::fs::read(&path).unwrap();
        let crlf = content.windows(2).filter(|w| w == b"\r\n").count();
        assert_eq!(
            crlf,
            0,
            "LF file must stay LF: {:?}",
            String::from_utf8_lossy(&content)
        );
    }

    #[tokio::test]
    async fn one_stray_crlf_does_not_convert_the_whole_file() {
        // A mostly-LF file with a single hand-edited CRLF line: line endings
        // are decided by majority, so the edit must not rewrite every ending.
        let dir = tempdir().unwrap();
        let path = dir.path().join("mixed.txt");
        std::fs::write(&path, "aaa\nbbb\r\nccc\nddd\n").unwrap();
        let out = EditFile
            .run(
                json!({"path": "mixed.txt", "old_text": "ddd", "new_text": "DDD"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("Edited"), "got: {}", out.text);
        let content = std::fs::read_to_string(&path).unwrap();
        let crlf = content.matches("\r\n").count();
        assert_eq!(
            crlf, 0,
            "LF-dominant file must stay LF, not become all-CRLF: {content:?}"
        );
    }

    #[tokio::test]
    async fn bom_file_matches_bomless_anchor_and_keeps_bom() {
        // The BOM is invisible in the rendered line 1, so a model-written
        // anchor for the first line must still match — and the saved file must
        // keep the BOM Visual Studio expects.
        let dir = tempdir().unwrap();
        let path = dir.path().join("bom.c");
        std::fs::write(&path, "\u{FEFF}int main(void)\n{\n}\n").unwrap();
        let out = EditFile
            .run(
                json!({"path": "bom.c", "old_text": "int main(void)", "new_text": "int app_main(void)"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(out.text.contains("Edited"), "got: {}", out.text);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.starts_with('\u{FEFF}'),
            "BOM must survive the edit: {content:?}"
        );
        assert_eq!(content, "\u{FEFF}int app_main(void)\n{\n}\n");
    }

    #[tokio::test]
    async fn hashline_edits_by_content_hash() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\nbbb\nccc\n").unwrap();
        let anchor = crate::tools::util::line_hash_prefix("bbb");
        let ok = EditFile
            .run(
                json!({"path": "a.txt", "hashline": anchor, "new_text": "XXX"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
        assert!(
            ok.text.contains("-bbb"),
            "diff should show removed line: {}",
            ok.text
        );
        assert!(
            ok.text.contains("+XXX"),
            "diff should show added line: {}",
            ok.text
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "aaa\nXXX\nccc\n"
        );
    }

    #[tokio::test]
    async fn hashline_range_edits_multiple_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\nbbb\nccc\nddd\n").unwrap();
        let start = crate::tools::util::line_hash_prefix("bbb");
        let end = crate::tools::util::line_hash_prefix("ccc");
        let ok = EditFile
            .run(
                json!({"path": "a.txt", "hashline": start, "end_hashline": end, "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "aaa\nX\nddd\n"
        );
    }

    #[tokio::test]
    async fn hashline_missing_anchor_reports_concurrent_change() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "hashline": "00000000", "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[ConcurrentChange]"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn hashline_ambiguous_anchor_is_rejected() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "bbb\nbbb\n").unwrap();
        let anchor = crate::tools::util::line_hash_prefix("bbb");
        let err = EditFile
            .run(
                json!({"path": "a.txt", "hashline": anchor, "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("not unique"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn hashline_empty_or_short_anchor_is_rejected() {
        // An empty anchor startswith-matches EVERY line; in a single-line
        // file it would "uniquely" resolve and edit with no anchor at all.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "only line\n").unwrap();
        for anchor in ["", "abc", "zzzzzzzz"] {
            let err = EditFile
                .run(
                    json!({"path": "a.txt", "hashline": anchor, "new_text": "X"}),
                    &ctx(dir.path()),
                )
                .await
                .unwrap_err();
            assert!(
                err.message.contains("[InvalidInput]"),
                "anchor {anchor:?}: {}",
                err.message
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "only line\n",
            "file must be untouched"
        );
    }

    #[tokio::test]
    async fn no_change_edit_is_a_hard_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "bbb\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "bbb", "new_text": "bbb"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("no change"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn expected_sha256_guards_against_stale_reads() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let digest = firment_core::hash::sha256_hex(b"hello\n");
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "hello", "new_text": "hi", "expected_sha256": "0".repeat(64)}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[ConcurrentChange]"),
            "got: {}",
            err.message
        );
        assert!(err.message.contains("current"), "got: {}", err.message);

        let ok = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "hello", "new_text": "hi", "expected_sha256": digest}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
    }

    #[tokio::test]
    async fn non_utf8_file_is_refused_not_corrupted() {
        // A GBK-encoded file must be refused wholesale: the lossy text
        // pipeline would otherwise rewrite every invalid byte as U+FFFD.
        let dir = tempdir().unwrap();
        let path = dir.path().join("gbk.txt");
        // "你好" in GBK
        let gbk = b"\xc4\xe3\xba\xc3\nworld\n".to_vec();
        std::fs::write(&path, &gbk).unwrap();
        let err = EditFile
            .run(
                json!({"path": "gbk.txt", "old_text": "world", "new_text": "WORLD"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[Encoding]"), "got: {}", err.message);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            gbk,
            "file bytes must be untouched"
        );
    }

    #[tokio::test]
    async fn invalid_anchor_returns_tagged_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "nope", "new_text": "x"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[InvalidInput]"),
            "got: {}",
            err.message
        );
    }

    /// The friction this feature exists for: the model typed an anchor that is
    /// one character off, and used to burn a whole round trip on "matched 0
    /// times".
    #[tokio::test]
    async fn a_near_miss_anchor_reports_the_closest_location_and_changes_nothing() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("main.c");
        std::fs::write(
            &file,
            "void setup(void) {\n  htim2.Init.Period = 999;\n  htim2.Init.Prescaler = 84;\n}\n",
        )
        .unwrap();

        let err = EditFile
            .run(
                // One character off: `999` vs `99`.
                json!({
                    "path": "main.c",
                    "old_text": "  htim2.Init.Period = 99;",
                    "new_text": "  htim2.Init.Period = 499;"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();

        assert!(
            err.message.contains("[InvalidInput]"),
            "got: {}",
            err.message
        );
        assert!(
            err.message.contains("Closest match"),
            "a 0-match failure must suggest a location: {}",
            err.message
        );
        assert!(
            err.message.contains("htim2.Init.Period = 999;"),
            "the suggestion must quote the real text: {}",
            err.message
        );
        assert!(
            err.message.contains("Nothing was changed"),
            "the model must be told nothing was applied: {}",
            err.message
        );
        // Never applied by itself -- that is the whole safety property.
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "void setup(void) {\n  htim2.Init.Period = 999;\n  htim2.Init.Prescaler = 84;\n}\n"
        );
    }

    #[tokio::test]
    async fn trailing_whitespace_differences_still_match_but_indentation_does_not() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.c");
        // The file line ends with two spaces; the anchor does not.
        std::fs::write(&file, "alpha\nbeta  \ngamma\n").unwrap();

        EditFile
            .run(
                json!({"path": "a.c", "old_text": "beta", "new_text": "BETA"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(
            // The anchor is matched as WHOLE LINES, so the replacement covers
            // the line's content including the trailing spaces the anchor did
            // not have. That is the tidier outcome: tolerating the difference
            // lets the line be FOUND, and replacing the line then removes the
            // stray whitespace instead of leaving `BETA  ` behind.
            std::fs::read_to_string(&file).unwrap(),
            "alpha\nBETA\ngamma\n"
        );

        // Leading whitespace is indentation: an anchor with the wrong indent is
        // a DIFFERENT piece of code, so it must not match. Edit line 1 of a file
        // where line 2 shares the text under an indent.
        std::fs::write(&file, "beta\n    beta\n").unwrap();
        EditFile
            .run(
                json!({"path": "a.c", "old_text": "    beta", "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\nX\n");
    }

    /// Python and Makefile care about trailing whitespace (a recipe line's tab,
    /// a continuation), so the tolerance is off there unless asked for.
    #[tokio::test]
    async fn whitespace_tolerance_is_off_for_indentation_significant_files() {
        let dir = tempdir().unwrap();
        let py = dir.path().join("m.py");
        std::fs::write(&py, "x = 1  \n").unwrap();

        let err = EditFile
            .run(
                json!({"path": "m.py", "old_text": "x = 1", "new_text": "x = 2"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("not found") || err.message.contains("Closest match"),
            "a .py file must not tolerate the difference by default: {}",
            err.message
        );
        assert_eq!(std::fs::read_to_string(&py).unwrap(), "x = 1  \n");

        // ...but the caller can opt in for that one call.
        EditFile
            .run(
                json!({
                    "path": "m.py",
                    "old_text": "x = 1",
                    "new_text": "x = 2",
                    "ignore_trailing_whitespace": true
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&py).unwrap(), "x = 2\n");
    }

    #[tokio::test]
    async fn batch_edits_apply_in_order_and_all_or_nothing() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();

        let out = EditFile
            .run(
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old_text": "one", "new_text": "1"},
                        {"old_text": "four", "new_text": "4"}
                    ]
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "1\ntwo\nthree\n4\n"
        );
        // One diff for the whole batch, not one per item.
        assert!(out.text.contains("@@"), "got: {}", out.text);

        // A later failure must leave the file exactly as it was: the first edit
        // is applied to an in-memory copy and never written.
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();
        let err = EditFile
            .run(
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old_text": "one", "new_text": "1"},
                        {"old_text": "does-not-exist", "new_text": "x"}
                    ]
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("edits[1]"),
            "a batch failure must name the failing item: {}",
            err.message
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "one\ntwo\nthree\nfour\n",
            "a failed batch must not be partially applied"
        );
    }

    #[tokio::test]
    async fn a_later_batch_edit_sees_the_earlier_ones_result() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "alpha\n").unwrap();

        // The second anchor only exists AFTER the first edit is applied, which
        // is what "in order, each seeing the previous result" has to mean.
        EditFile
            .run(
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old_text": "alpha", "new_text": "beta"},
                        {"old_text": "beta", "new_text": "gamma"}
                    ]
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "gamma\n");
    }

    #[tokio::test]
    async fn batch_and_single_edit_fields_are_mutually_exclusive() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        let err = EditFile
            .run(
                json!({
                    "path": "a.txt",
                    "old_text": "x",
                    "new_text": "y",
                    "edits": [{"old_text": "x", "new_text": "y"}]
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("not both"),
            "mixing the two shapes is ambiguous and must be refused: {}",
            err.message
        );
    }

    #[test]
    fn schema_describes_batch_mode_and_the_whitespace_switch() {
        let schema = EditFile.input_schema();
        let props = &schema["properties"];
        assert!(
            props.get("edits").is_some(),
            "the schema must advertise batch mode"
        );
        assert!(
            props.get("ignore_trailing_whitespace").is_some(),
            "the whitespace switch must be discoverable"
        );
        // Four accepted shapes now: three single-edit modes plus the batch.
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 4);
        // The batch items carry their own mode discriminator.
        let items = &props["edits"]["items"];
        assert_eq!(items["oneOf"].as_array().unwrap().len(), 3);
        assert_eq!(items["required"][0], "new_text");
    }
}
