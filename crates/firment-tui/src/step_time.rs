//! How long a step took, and what it is likely to take next time.
//!
//! The rule, and the same one the GUI keeps in `gui/src/lib/timing.ts`: **a
//! duration on screen is either measured or absent.** A finished card reports what
//! it took; a running one counts up; `~4.0s` is offered beside a running card only
//! when this session has already watched the same tool finish at least twice, which
//! makes it a median of real runs rather than a guess with a unit stuck on it.
//!
//! What it deliberately does not do: forecast the turn, forecast a step that has not
//! started, or carry a table of "typical" durations. A bar that promises twelve
//! seconds and takes forty is worse than no bar -- the project's position is that an
//! unverifiable number is a defect, not a nicety.
//!
//! The two surfaces keep separate ledgers on purpose: neither can see the other's
//! clock, and a shared one would have to be plumbed through the kernel for a label.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Completed runs per tool name, oldest first.
pub(crate) type Runs = HashMap<String, Vec<Duration>>;

/// How many runs a median is drawn from. Eight keeps it responsive to a project
/// whose builds just got slower without letting one outlier from this morning
/// dominate an afternoon of work.
pub(crate) const RUNS_KEPT: usize = 8;

/// Two runs, not one. A single observation is not a median -- it is that run, and
/// printing it back would be the tool telling the user what they just watched.
const RUNS_FOR_ESTIMATE: usize = 2;

/// Above this, a tenth of a second stops meaning anything. The value is 9 950ms
/// rather than 10 000 so that a duration which *rounds* to ten seconds takes the
/// whole-second branch: `10.0s` and `10s` must not both be reachable.
const TENTHS_CEILING: Duration = Duration::from_millis(9_950);

/// Count one finished run.
pub(crate) fn record(runs: &mut Runs, name: &str, took: Duration) {
    let list = runs.entry(name.to_string()).or_default();
    list.push(took);
    let excess = list.len().saturating_sub(RUNS_KEPT);
    if excess > 0 {
        list.drain(..excess);
    }
}

/// The median of this session's completed runs of `name`, or `None` when there is
/// not enough history to call it one.
///
/// A median rather than a mean: one build that hit a cold page cache is exactly the
/// sample a mean would let speak for the tool.
pub(crate) fn estimate(runs: &Runs, name: &str) -> Option<Duration> {
    let list = runs.get(name)?;
    if list.len() < RUNS_FOR_ESTIMATE {
        return None;
    }
    let mut sorted = list.clone();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2
    })
}

/// The measured time for one card: so far while it runs, in total once it has ended.
///
/// `waited` is the time a person spent at this call's permission gate, which is
/// subtracted because the number on screen answers "how long did the tool take",
/// not "how long was this card on screen". A running card has no `waited` yet —
/// the person has not answered — so it counts wall time until it ends and corrects
/// to the tool's own time in the same update that clears `running`.
///
/// `None` for a card with no clock at all -- a tool reopened from a stored
/// transcript, which records that it was called and nothing about how long it took.
pub(crate) fn measured(
    started: Option<Instant>,
    ended: Option<Instant>,
    waited: Option<Duration>,
    now: Instant,
) -> Option<Duration> {
    let started = started?;
    let wall = match ended {
        Some(end) => end.saturating_duration_since(started),
        None => now.saturating_duration_since(started),
    };
    // `saturating` because a wait cannot outlast its own call; if a report ever
    // says it did, the honest answer is "the tool took no measurable time", not a
    // panic and not a negative duration.
    Some(wall.saturating_sub(waited.unwrap_or_default()))
}

/// A step's duration, at the precision its reader needs.
///
/// One decimal under ten seconds, whole seconds from there, then minutes and hours
/// in the shape `util::format_ts` uses nearby -- so two labels on screen cannot spell
/// the same duration two ways. A whole-second formatter would print `0s` for a flash
/// that took 400ms, which reads as "nothing happened".
pub(crate) fn format_step_duration(took: Duration) -> String {
    let ms = took.as_millis();
    // Anything under a tenth of a second is one fact, not two, so it is spelled
    // once rather than printed as `0.0s`.
    if ms < 100 {
        return "<0.1s".to_string();
    }
    if took < TENTHS_CEILING {
        let tenths = (ms as f64 / 100.0).round() / 10.0;
        if tenths < 10.0 {
            return format!("{tenths:.1}s");
        }
    }

    // Rounded before the split, or 9 999ms would print `10.0s` here and `10s` one
    // millisecond later.
    let secs = (ms + 500) / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let minutes = secs / 60;
    let remainder = secs % 60;
    if minutes < 60 {
        return format!("{minutes}m {remainder:02}s");
    }
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs_of(pairs: &[(&str, u64)]) -> Runs {
        let mut runs = Runs::new();
        for (name, ms) in pairs {
            record(&mut runs, name, Duration::from_millis(*ms));
        }
        runs
    }

    #[test]
    fn one_run_is_not_a_median() {
        let runs = runs_of(&[("build", 4_000)]);
        assert_eq!(estimate(&runs, "build"), None);
        assert_eq!(estimate(&runs, "flash"), None, "no history at all");
    }

    #[test]
    fn the_median_ignores_one_cold_build() {
        let runs = runs_of(&[("build", 4_000), ("build", 90_000), ("build", 4_200)]);
        // A mean would be 32.7s here, which is a duration nothing in this session
        // ever took.
        assert_eq!(estimate(&runs, "build"), Some(Duration::from_millis(4_200)));
    }

    #[test]
    fn an_even_count_averages_the_middle_pair() {
        let runs = runs_of(&[("build", 4_000), ("build", 5_000)]);
        assert_eq!(estimate(&runs, "build"), Some(Duration::from_millis(4_500)));
    }

    #[test]
    fn the_window_keeps_the_recent_runs() {
        let mut runs = Runs::new();
        for i in 1..=10u64 {
            record(&mut runs, "build", Duration::from_millis(i * 1_000));
        }
        // Ten offered, eight kept, oldest dropped: the median of 3s..10s is 6.5s
        // where all ten would have given 5.5s. Pinned rather than described, so the
        // direction of the window is a decision someone has to change on purpose.
        assert_eq!(runs["build"].len(), RUNS_KEPT);
        assert_eq!(estimate(&runs, "build"), Some(Duration::from_millis(6_500)));
    }

    #[test]
    fn tools_keep_their_own_history() {
        let runs = runs_of(&[
            ("build", 1_000),
            ("build", 3_000),
            ("flash", 20_000),
            ("flash", 30_000),
        ]);
        assert_eq!(estimate(&runs, "build"), Some(Duration::from_millis(2_000)));
        assert_eq!(
            estimate(&runs, "flash"),
            Some(Duration::from_millis(25_000))
        );
    }

    #[test]
    fn a_card_with_no_clock_has_no_duration() {
        let now = Instant::now();
        assert_eq!(
            measured(None, None, None, now),
            None,
            "reopened transcript card"
        );
        let started = now;
        assert_eq!(
            measured(Some(started), None, None, now + Duration::from_secs(3)),
            Some(Duration::from_secs(3))
        );
        // Once it has ended, the clock stops mattering: asking again later gives the
        // same answer rather than a growing one.
        let ended = now + Duration::from_secs(4);
        assert_eq!(
            measured(
                Some(started),
                Some(ended),
                None,
                now + Duration::from_secs(90)
            ),
            Some(Duration::from_secs(4))
        );
    }

    #[test]
    fn a_persons_reading_time_belongs_to_nobody_on_the_card() {
        // Two minutes of the user deciding, eight seconds of `flash`. The card has to
        // say eight seconds, because that is the number the next `flash` will be
        // estimated from and the number that says whether the tool was slow.
        let now = Instant::now();
        let started = now;
        let ended = now + Duration::from_secs(128);
        assert_eq!(
            measured(
                Some(started),
                Some(ended),
                Some(Duration::from_secs(120)),
                now
            ),
            Some(Duration::from_secs(8))
        );
        // No report, no subtraction: an auto-approved call keeps its whole clock.
        assert_eq!(
            measured(Some(started), Some(ended), None, now),
            Some(Duration::from_secs(128))
        );
        // A report that cannot be true (the wait longer than the call) still cannot
        // produce a negative duration on screen.
        assert_eq!(
            measured(
                Some(started),
                Some(ended),
                Some(Duration::from_secs(600)),
                now
            ),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_step_keeps_a_tenth_where_a_whole_second_would_read_as_nothing() {
        assert_eq!(format_step_duration(Duration::from_millis(0)), "<0.1s");
        assert_eq!(format_step_duration(Duration::from_millis(99)), "<0.1s");
        assert_eq!(format_step_duration(Duration::from_millis(400)), "0.4s");
        assert_eq!(format_step_duration(Duration::from_millis(4_234)), "4.2s");
    }

    #[test]
    fn the_switch_to_whole_seconds_happens_once() {
        assert_eq!(format_step_duration(Duration::from_millis(9_900)), "9.9s");
        assert_eq!(format_step_duration(Duration::from_millis(9_999)), "10s");
        assert_eq!(format_step_duration(Duration::from_millis(10_000)), "10s");
        assert_eq!(format_step_duration(Duration::from_secs(65)), "1m 05s");
        assert_eq!(format_step_duration(Duration::from_secs(3_600)), "1h 00m");
    }
}
