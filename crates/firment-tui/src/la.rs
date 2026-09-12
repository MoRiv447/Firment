//! The logic analyzer's last measurement, and a schematic of it.
//!
//! **Not a capture.** The raw samples live in sigrok `.sr` sessions under
//! `.firment/la/`, and the `la` tool's own contract says measurements never
//! parse them. What arrives here is the tool's measurement text, so the most a
//! panel can honestly draw is the *shape the numbers describe* -- one period at
//! the measured duty -- and it is labelled as a schematic for that reason. A
//! drawing presented as a capture would be the most convincing kind of wrong.
//!
//! Parsing text is the weaker option and it is the one available: the tool's
//! result reaches the UI as text, and inventing a second structured channel for
//! it would mean two representations of the same measurement, which is the
//! drift this codebase keeps refusing. The parser is therefore narrow and
//! forgiving in the right direction -- anything it cannot find stays `None` and
//! the panel simply does not claim it.

/// One measurement, as far as the tool's text lets us know it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LaReading {
    /// The channel the measurement was taken on, from the header line.
    pub(crate) channel: String,
    /// Frequency as a range, which is what the tool reports: a range plus a
    /// typical value, never a single number pretending to be exact.
    pub(crate) low_hz: Option<f64>,
    pub(crate) high_hz: Option<f64>,
    /// Percent of the period spent high.
    pub(crate) duty_pct: Option<f64>,
    pub(crate) rising_edges: Option<usize>,
    /// `low` / `medium` / `high`, from the tool's own confidence.
    pub(crate) confidence: Option<String>,
}

impl LaReading {
    /// Whether anything was actually measured. A header with no numbers is not
    /// a reading and must not be shown as one.
    pub(crate) fn is_empty(&self) -> bool {
        self.low_hz.is_none() && self.duty_pct.is_none() && self.rising_edges.is_none()
    }

    /// The frequency range as one string, e.g. `998 .. 1002 Hz`.
    pub(crate) fn frequency(&self) -> Option<String> {
        match (self.low_hz, self.high_hz) {
            (Some(low), Some(high)) if low == high => Some(format!("{low:.0} Hz")),
            (Some(low), Some(high)) => Some(format!("{low:.0} .. {high:.0} Hz")),
            _ => None,
        }
    }
}

/// The value of a `  label: rest` line, if the text has one.
fn field<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.lines()
        .map(str::trim_start)
        .find_map(|line| line.strip_prefix(label))
        .map(|rest| rest.trim_start_matches([':', ' ']).trim())
}

/// Parse the `la` tool's measure output, or `None` if this is not one.
///
/// The header check matters: `capture`, `info`, `detect` and `decode` produce
/// text too, and none of them is a measurement. Parsing one of those as a
/// reading would put a probe's capabilities in the panel as if they had been
/// observed.
pub(crate) fn parse_measure(text: &str) -> Option<LaReading> {
    if !text.contains("[la] measure") {
        return None;
    }
    let channel = text
        .split_whitespace()
        .find_map(|token| token.strip_prefix("channel="))
        .unwrap_or("")
        .to_string();

    let mut reading = LaReading {
        channel,
        ..Default::default()
    };

    // `frequency: 998 .. 1002 Hz (~1000.00)` -- the range, not the midpoint, is
    // what the tool is willing to claim.
    if let Some(rest) = field(text, "frequency:")
        && let Some((low, high)) = rest.split_once("..")
    {
        reading.low_hz = low.trim().parse().ok();
        reading.high_hz = high.split_whitespace().next().and_then(|n| n.parse().ok());
    }
    if let Some(rest) = field(text, "duty:") {
        reading.duty_pct = rest.split('%').next().and_then(|n| n.trim().parse().ok());
    }
    if let Some(rest) = field(text, "rising edges:") {
        reading.rising_edges = rest.split_whitespace().next().and_then(|n| n.parse().ok());
    }
    if let Some(rest) = field(text, "confidence:") {
        // `high — note about why`
        let name = rest.split(['—', '-']).next().unwrap_or(rest).trim();
        if !name.is_empty() {
            reading.confidence = Some(name.to_string());
        }
    }

    if reading.is_empty() {
        // A measure that found nothing (`not periodic`) is not a reading, and
        // showing it as one would claim a period nobody observed.
        return None;
    }
    Some(reading)
}

/// One period of a square wave at a duty cycle, as `cells` blocks.
///
/// High cells first: this is a *shape*, not a window onto time, so there is no
/// phase to get right and no start time to be honest about.
pub(crate) fn schematic(duty_pct: Option<f64>, cells: usize) -> String {
    // 50% when the duty is unknown: the shape of a signal whose duty was never
    // measured is not known, and half is the least misleading default.
    let duty = duty_pct.unwrap_or(50.0).clamp(0.0, 100.0);
    let high = ((duty / 100.0) * cells as f64).round() as usize;
    (0..cells)
        .map(|i| if i < high { '▇' } else { '▁' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real measure output, copied from the tool's own format strings.
    const FREQUENCY: &str = "[la] measure capture=1k-pwm channel=0 (frequency)\n  samples: 8000\n  \
                             frequency: 998 .. 1002 Hz (~1000.00)\n  rising edges: 10\n  \
                             confidence: high — the period repeated exactly\n[evidence: physical \
                             — logic capture]";

    const DUTY: &str = "[la] measure capture=1k-pwm channel=1 (duty)\n  samples: 8000\n  duty: \
                        45.0% high\n  confidence: medium — only ten periods\n";

    #[test]
    fn a_frequency_measurement_is_read_as_a_range() {
        let r = parse_measure(FREQUENCY).expect("a measurement");
        assert_eq!(r.channel, "0");
        assert_eq!(r.low_hz, Some(998.0));
        assert_eq!(r.high_hz, Some(1002.0));
        assert_eq!(r.rising_edges, Some(10));
        assert_eq!(r.frequency().as_deref(), Some("998 .. 1002 Hz"));
        assert_eq!(r.confidence.as_deref(), Some("high"));
        // The frequency action does not report a duty, and the panel must not
        // invent one.
        assert_eq!(r.duty_pct, None);
    }

    #[test]
    fn a_duty_measurement_carries_its_percentage() {
        let r = parse_measure(DUTY).expect("a measurement");
        assert_eq!(r.channel, "1");
        assert_eq!(r.duty_pct, Some(45.0));
        assert_eq!(r.confidence.as_deref(), Some("medium"));
        assert_eq!(r.frequency(), None);
    }

    #[test]
    fn a_capture_is_not_a_measurement() {
        // The other four actions produce text too, and none of them measured
        // anything. Parsing one as a reading would present a capability as an
        // observation.
        let capture = "[la] capture capture=1k-pwm channels=0,1 samples=8000\n  saved: \
                       .firment/la/1k-pwm.sr\n";
        assert!(parse_measure(capture).is_none());
    }

    #[test]
    fn a_measurement_that_found_nothing_is_not_a_reading() {
        let not_periodic = "[la] measure capture=x channel=0 (frequency)\n  samples: 40\n  \
                            frequency: not periodic (fewer than two rising edges — one \
                            transition is not a period)\n";
        assert!(parse_measure(not_periodic).is_none());
    }

    #[test]
    fn a_single_frequency_is_not_shown_as_a_range() {
        let r = LaReading {
            low_hz: Some(1000.0),
            high_hz: Some(1000.0),
            ..Default::default()
        };
        assert_eq!(r.frequency().as_deref(), Some("1000 Hz"));
    }

    #[test]
    fn the_schematic_draws_the_duty_it_was_given() {
        assert_eq!(schematic(Some(50.0), 8), "▇▇▇▇▁▁▁▁");
        assert_eq!(schematic(Some(25.0), 8), "▇▇▁▁▁▁▁▁");
        assert_eq!(schematic(Some(100.0), 4), "▇▇▇▇");
        // Not measured: half, rather than a shape nobody observed.
        assert_eq!(schematic(None, 4), "▇▇▁▁");
    }

    #[test]
    fn the_schematic_is_always_exactly_the_width_asked_for() {
        // It is drawn into a fixed column, so an off-by-one would misalign the
        // panel rather than merely look wrong.
        for duty in [0.0, 1.0, 33.3, 50.0, 99.9, 100.0] {
            assert_eq!(schematic(Some(duty), 18).chars().count(), 18, "duty {duty}");
        }
        assert_eq!(schematic(Some(50.0), 0), "");
    }
}
