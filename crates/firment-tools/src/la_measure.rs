//! Pure waveform measurements for the logic-analyzer tool (`la`).
//!
//! Same philosophy as `observe.rs`: the analysers are pure functions over
//! sample vectors — no IO, no clock, fully deterministic tests. The capture
//! layer (`tools/la.rs`) turns sigrok-cli's binary output into these vectors
//! via `la_cmd::unpack_bitstream`; nothing here ever touches a device.
//!
//! Every frequency-style verdict is a RANGE, never a bare number: edges are
//! only located to within one sample interval, so a point estimate would be
//! false precision (the blink lesson from observe.rs).

use crate::tools::observe::Confidence;

/// One digital channel: one sample per element, 0 or 1.
pub type Wave = [u8];

/// Which transitions to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Rising,
    Falling,
    Both,
}

/// Indices where the wave goes 0 -> 1 (the sample AT the index is high).
fn rising_indices(wave: &Wave) -> Vec<usize> {
    wave.windows(2)
        .enumerate()
        .filter(|(_, p)| p[0] == 0 && p[1] != 0)
        .map(|(i, _)| i + 1)
        .collect()
}

/// Indices of every transition, either direction.
fn edge_indices(wave: &Wave) -> Vec<usize> {
    wave.windows(2)
        .enumerate()
        .filter(|(_, p)| (p[0] == 0) != (p[1] == 0))
        .map(|(i, _)| i + 1)
        .collect()
}

/// Count transitions of the requested kind.
pub fn count_edges(wave: &Wave, kind: EdgeKind) -> usize {
    match kind {
        EdgeKind::Rising => rising_indices(wave).len(),
        EdgeKind::Falling => wave.windows(2).filter(|p| p[0] != 0 && p[1] == 0).count(),
        EdgeKind::Both => edge_indices(wave).len(),
    }
}

/// Frequency verdict over a periodic wave.
#[derive(Debug, Clone)]
pub struct Frequency {
    pub hz: f64,
    /// The honest answer: edges carry ±1 sample of localisation error, so
    /// the period carries ±2 and the frequency is a range.
    pub hz_low: f64,
    pub hz_high: f64,
    pub rising_edges: usize,
    pub confidence: Confidence,
    pub note: String,
}

/// Fewer samples per period than this and the measurement is aliasing
/// territory: the verdict is reported, but at LOW confidence.
const MIN_SAMPLES_PER_PERIOD: f64 = 4.0;

/// The edge set a period is measured from: rising edges normally, falling
/// edges as fallback. A square wave that STARTS high (`11110000…`) has one
/// rising edge per two periods — refusing it because "rising < 2" would
/// throw away a perfectly periodic wave. Either edge family spaced evenly
/// still defines the period.
fn period_edges(wave: &Wave) -> (Vec<usize>, &'static str) {
    let rising = rising_indices(wave);
    if rising.len() >= 2 {
        return (rising, "rising");
    }
    let falling: Vec<usize> = wave
        .windows(2)
        .enumerate()
        .filter(|(_, p)| p[0] != 0 && p[1] == 0)
        .map(|(i, _)| i + 1)
        .collect();
    if falling.len() >= 2 {
        return (falling, "falling");
    }
    (rising, "rising")
}

/// Measure the repetition rate of a periodic wave. Needs at least TWO edges
/// of one family — one edge is a transition, not a period (the observe
/// blink rule). Returns `None` when no period can be claimed.
pub fn measure_frequency(wave: &Wave, samplerate_hz: f64) -> Option<Frequency> {
    if samplerate_hz <= 0.0 {
        return None;
    }
    let (edges, family) = period_edges(wave);
    let n = edges.len();
    if n < 2 {
        return None;
    }
    let period = (edges[n - 1] - edges[0]) as f64 / (n - 1) as f64;
    if period <= 0.0 {
        return None;
    }
    let hz = samplerate_hz / period;
    let hz_low = samplerate_hz / (period + 2.0);
    // Never claim above Nyquist even when the error bar would allow it.
    let hz_high = samplerate_hz / (period - 2.0).max(2.0);
    let undersampled = period < MIN_SAMPLES_PER_PERIOD;
    let confidence = if undersampled {
        Confidence::Low
    } else if n >= 4 {
        Confidence::High
    } else {
        Confidence::Medium
    };
    let note = if undersampled {
        format!(
            "{period:.1} samples per period at {samplerate_hz:.0} Hz — under {MIN_SAMPLES_PER_PERIOD:.0}x \
             oversampling the range is all this wave can honestly support"
        )
    } else {
        format!("{n} {family} edges, {period:.1} samples per period")
    };
    Some(Frequency {
        hz,
        hz_low,
        hz_high,
        rising_edges: n,
        confidence,
        note,
    })
}

/// Duty-cycle verdict: fraction of complete-period samples spent high.
#[derive(Debug, Clone)]
pub struct Duty {
    pub fraction: f64,
    pub rising_edges: usize,
    pub confidence: Confidence,
    pub note: String,
}

/// Measure duty cycle over the COMPLETE periods between the first and last
/// edge of the chosen family — a trailing partial period would skew the
/// fraction. The fraction is dimensionless, so no time base is needed; the
/// confidence still cares about samples per period.
pub fn measure_duty(wave: &Wave) -> Option<Duty> {
    let (edges, _) = period_edges(wave);
    let n = edges.len();
    if n < 2 {
        return None;
    }
    let span = &wave[edges[0]..edges[n - 1]];
    if span.is_empty() {
        return None;
    }
    let high = span.iter().filter(|&&s| s != 0).count() as f64;
    let fraction = high / span.len() as f64;
    let period = span.len() as f64 / (n - 1) as f64;
    let confidence = if period < MIN_SAMPLES_PER_PERIOD {
        Confidence::Low
    } else if n >= 4 {
        Confidence::High
    } else {
        Confidence::Medium
    };
    Some(Duty {
        fraction,
        rising_edges: n,
        confidence,
        note: format!(
            "{:.1}% high over {} complete period(s)",
            fraction * 100.0,
            n - 1
        ),
    })
}

/// High-pulse width extremes.
#[derive(Debug, Clone)]
pub struct PulseWidths {
    pub min_ns: f64,
    pub max_ns: f64,
    pub pulses: usize,
    pub confidence: Confidence,
    pub note: String,
}

/// Widths of the high runs. A run still open at the window edge is TRUNCATED
/// by the capture, not real — counting it would report a false maximum. A
/// shortest pulse under [`MIN_SAMPLES_PER_PERIOD`] samples is at the edge of
/// what the sample rate can resolve, so the min is reported at LOW
/// confidence.
pub fn measure_pulse_widths(wave: &Wave, samplerate_hz: f64) -> Option<PulseWidths> {
    if samplerate_hz <= 0.0 {
        return None;
    }
    let mut pulses: Vec<usize> = Vec::new();
    let mut run = 0usize;
    for &s in wave {
        if s != 0 {
            run += 1;
        } else if run > 0 {
            pulses.push(run);
            run = 0;
        }
    }
    // `run > 0` here means the wave ended mid-pulse: truncated, discarded.
    if pulses.is_empty() {
        return None;
    }
    let min = *pulses.iter().min().unwrap();
    let max = *pulses.iter().max().unwrap();
    let ns = |samples: usize| samples as f64 * 1e9 / samplerate_hz;
    let confidence = if (min as f64) < MIN_SAMPLES_PER_PERIOD {
        Confidence::Low
    } else {
        Confidence::High
    };
    Some(PulseWidths {
        min_ns: ns(min),
        max_ns: ns(max),
        pulses: pulses.len(),
        confidence,
        note: format!("{} high pulse(s), {min}..{max} samples wide", pulses.len()),
    })
}

/// Rough serial bitrate estimate from the median gap between transitions.
#[derive(Debug, Clone)]
pub struct BitRate {
    pub bps: f64,
    pub confidence: Confidence,
    pub note: String,
}

/// Estimate a line bitrate as `samplerate / median gap between adjacent
/// transitions`. A heuristic for "what baud is this UART" — the median
/// survives a few stretched or glued edges, but it assumes transitions
/// happen at bit boundaries, so it is a starting point for a real decode,
/// not a measurement of one. LOW confidence when the median gap is under
/// [`MIN_SAMPLES_PER_PERIOD`] samples.
pub fn estimate_bitrate(wave: &Wave, samplerate_hz: f64) -> Option<BitRate> {
    if samplerate_hz <= 0.0 {
        return None;
    }
    let edges = edge_indices(wave);
    if edges.len() < 2 {
        return None;
    }
    let mut gaps: Vec<usize> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2] as f64;
    if median <= 0.0 {
        return None;
    }
    let bps = samplerate_hz / median;
    let confidence = if median < MIN_SAMPLES_PER_PERIOD {
        Confidence::Low
    } else {
        Confidence::Medium
    };
    Some(BitRate {
        bps,
        confidence,
        note: format!(
            "median {median:.0}-sample gap between {} transitions — heuristic, confirm with a \
             protocol decode",
            edges.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Square wave: `period` samples per full cycle, `duty_high` of them
    /// high, starting high at index 0.
    fn square(period: usize, cycles: usize, duty_high: usize) -> Vec<u8> {
        let mut w = Vec::with_capacity(period * cycles);
        for _ in 0..cycles {
            for i in 0..period {
                w.push(if i < duty_high { 1 } else { 0 });
            }
        }
        w
    }

    #[test]
    fn frequency_of_1khz_square_at_8mhz() {
        // 8000 samples per period = 1 kHz at 8 MHz; five cycles so the
        // four rising edges earn a HIGH-confidence verdict.
        let w = square(8000, 5, 4000);
        let f = measure_frequency(&w, 8e6).expect("a periodic wave has a frequency");
        assert!(
            f.hz_low <= 1000.0 && 1000.0 <= f.hz_high,
            "1 kHz inside {}..{}",
            f.hz_low,
            f.hz_high
        );
        assert!((f.hz - 1000.0).abs() < 1.0);
        assert_eq!(f.confidence.name(), "HIGH");
    }

    #[test]
    fn one_rising_edge_is_a_transition_not_a_period() {
        let mut w = vec![0u8; 100];
        for s in w.iter_mut().skip(50) {
            *s = 1;
        }
        assert!(
            measure_frequency(&w, 1e6).is_none(),
            "a single power-on edge must not yield a frequency"
        );
    }

    #[test]
    fn leading_high_square_measures_via_falling_edges() {
        // 11110000 11110000: exactly ONE rising edge (index 8) but two
        // falling edges — refusing it for "rising < 2" would throw away a
        // perfectly periodic wave.
        let w = [1u8, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0];
        let f = measure_frequency(&w, 8e6).expect("falling edges define the period");
        assert!((f.hz - 1e6).abs() < 1.0, "got {}", f.hz);
        assert!(f.note.contains("falling"), "got: {}", f.note);
    }

    #[test]
    fn flat_wave_has_no_frequency() {
        let w = vec![1u8; 64];
        assert!(measure_frequency(&w, 1e6).is_none());
        let w = vec![0u8; 64];
        assert!(measure_frequency(&w, 1e6).is_none());
        // And it must not panic on the empty wave either.
        assert!(measure_frequency(&[], 1e6).is_none());
    }

    #[test]
    fn undersampled_wave_is_low_confidence() {
        // 2 samples per period at 8 MHz = 4 MHz square: aliasing territory.
        let w = square(2, 10, 1);
        let f = measure_frequency(&w, 8e6).expect("edges exist");
        assert_eq!(f.confidence.name(), "LOW");
        // The range must still bracket the point estimate and never exceed
        // Nyquist.
        assert!(f.hz_low <= f.hz && f.hz <= f.hz_high);
        assert!(f.hz_high <= 8e6 / 2.0 + f64::EPSILON);
    }

    #[test]
    fn duty_of_20_percent_square() {
        let w = square(100, 5, 20);
        let d = measure_duty(&w).expect("periodic wave");
        assert!((d.fraction - 0.2).abs() < 1e-9, "got {}", d.fraction);
        assert_eq!(d.confidence.name(), "HIGH");
    }

    #[test]
    fn duty_ignores_the_trailing_partial_period() {
        // Three 100-sample periods (20% high) plus 60 trailing low samples:
        // the partial tail must not drag the fraction down.
        let mut w = square(100, 3, 20);
        w.extend(std::iter::repeat_n(0u8, 60));
        let d = measure_duty(&w).expect("periodic wave");
        assert!((d.fraction - 0.2).abs() < 1e-9, "got {}", d.fraction);
    }

    #[test]
    fn edge_counts_per_kind() {
        // 1 0 1 0 1: rising at 2 and 4, falling at 3 and 5.
        let w = [1u8, 0, 1, 0, 1];
        assert_eq!(count_edges(&w, EdgeKind::Rising), 2);
        assert_eq!(count_edges(&w, EdgeKind::Falling), 2);
        assert_eq!(count_edges(&w, EdgeKind::Both), 4);
    }

    #[test]
    fn pulse_widths_find_the_glitch() {
        // A 10-sample pulse, then a 1-sample glitch, at 10 MHz.
        let mut w = vec![0u8; 40];
        for s in w.iter_mut().take(10) {
            *s = 1;
        }
        w[20] = 1;
        let p = measure_pulse_widths(&w, 1e7).expect("pulses exist");
        assert_eq!(p.pulses, 2);
        assert!(
            (p.min_ns - 100.0).abs() < 1e-6,
            "1 sample @ 10 MHz = 100 ns"
        );
        assert!((p.max_ns - 1000.0).abs() < 1e-6);
        assert_eq!(
            p.confidence.name(),
            "LOW",
            "a sub-4-sample min pulse is not trustworthy"
        );
    }

    #[test]
    fn bitrate_estimate_from_median_gap() {
        // Transitions every 87 samples at 1.8576 MHz ≈ 21352 bps.
        let mut w = Vec::with_capacity(87 * 12 + 1);
        for i in 0..12usize {
            w.extend(std::iter::repeat_n(if i % 2 == 0 { 1u8 } else { 0 }, 87));
        }
        let b = estimate_bitrate(&w, 1_857_600.0).expect("edges exist");
        assert!((b.bps - 21351.7).abs() < 1.0, "got {}", b.bps);
    }

    #[test]
    fn zero_samplerate_never_divides() {
        let w = square(10, 3, 5);
        assert!(measure_frequency(&w, 0.0).is_none());
        assert!(measure_pulse_widths(&w, 0.0).is_none());
        assert!(estimate_bitrate(&w, 0.0).is_none());
    }
}
