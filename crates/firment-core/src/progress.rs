//! Phase progress for long tools (plan §6-10, item 7).
//!
//! **Coarse phases, not byte counting.** The tools that take minutes are other programs —
//! `probe-rs`, the toolchain, a linker — and their own output is the only signal available. A
//! counter that claims a percentage it cannot know is worse than a phase that says what is
//! happening, so `current`/`total` are optional by shape (`0` means "unknown") rather than by
//! convention.
//!
//! The UI side of the plan's §16.2 constraint — a progress bar is not shown for a run shorter
//! than two seconds — lives in the frontends, not here. This module reports; it does not decide
//! what deserves to be drawn.

use std::sync::Arc;

/// One step of a long-running tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    /// What is happening, in the tool's own words (`erasing`, `linking`, `verify`).
    pub phase: String,
    /// How far along, when the tool knows. `0` with `total == 0` means "a phase, no count".
    pub current: u64,
    pub total: u64,
    /// Milliseconds remaining, when it can be estimated from the rate so far.
    pub eta_ms: Option<u64>,
}

impl ProgressEvent {
    /// A phase with no count behind it.
    pub fn phase(phase: impl Into<String>) -> Self {
        ProgressEvent {
            phase: phase.into(),
            current: 0,
            total: 0,
            eta_ms: None,
        }
    }

    /// A phase with a count, and an ETA extrapolated from the time it has taken so far.
    pub fn counted(phase: impl Into<String>, current: u64, total: u64, elapsed_ms: u64) -> Self {
        ProgressEvent {
            phase: phase.into(),
            current,
            total,
            eta_ms: eta_ms(elapsed_ms, current, total),
        }
    }
}

/// Estimate the remaining time from the rate so far.
///
/// `None` unless there is something real to extrapolate from: a total, some progress, and some
/// time. Below a few percent the rate is mostly startup cost, and an ETA that jumps from two
/// minutes to five seconds is worse than an absent one — it teaches the reader to ignore the
/// number, which is the whole value of having it.
pub fn eta_ms(elapsed_ms: u64, current: u64, total: u64) -> Option<u64> {
    if total == 0 || current == 0 || current >= total || elapsed_ms == 0 {
        return None;
    }
    // 5% or 250 ms of work, whichever comes first: a short run cannot have a usable rate, and a
    // long one should not wait for 5% of it to say anything.
    let enough_work = current.saturating_mul(20) >= total;
    if !enough_work && elapsed_ms < 250 {
        return None;
    }
    let remaining = total - current;
    Some(elapsed_ms.saturating_mul(remaining) / current)
}

/// The handle a tool reports through.
///
/// Carries the call's identity, so a frontend can put the phase on the card it belongs to
/// instead of in a global corner — and so a progress line from a subagent's tool is not
/// indistinguishable from one of the turn's own. Cheap to clone; a context without one means
/// nobody is listening, which is every direct tool run and every test.
/// Where a report goes: `(tool, seq, event)`.
///
/// Named rather than written inline for the reason clippy gives — and because the name is the
/// place to say what the three arguments are, which an `Arc<dyn Fn …>` spelled out does not.
pub type ProgressSink = Arc<dyn Fn(&str, u64, ProgressEvent) + Send + Sync>;

#[derive(Clone)]
pub struct ProgressReporter {
    tool: Arc<str>,
    seq: u64,
    sink: ProgressSink,
}

impl ProgressReporter {
    pub fn new(tool: impl Into<Arc<str>>, seq: u64, sink: ProgressSink) -> Self {
        ProgressReporter {
            tool: tool.into(),
            seq,
            sink,
        }
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Report a phase with no count.
    pub fn phase(&self, phase: impl Into<String>) {
        (self.sink)(&self.tool, self.seq, ProgressEvent::phase(phase));
    }

    /// Report a counted phase, computing the ETA from `elapsed_ms`.
    pub fn counted(&self, phase: impl Into<String>, current: u64, total: u64, elapsed_ms: u64) {
        (self.sink)(
            &self.tool,
            self.seq,
            ProgressEvent::counted(phase, current, total, elapsed_ms),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn an_eta_needs_something_to_extrapolate_from() {
        // Each `None` here is a case where a guess would be worse than silence.
        assert_eq!(eta_ms(1_000, 5, 0), None, "no total");
        assert_eq!(eta_ms(1_000, 0, 100), None, "nothing done");
        assert_eq!(eta_ms(0, 5, 100), None, "no time passed");
        assert_eq!(eta_ms(1_000, 100, 100), None, "already done");
        assert_eq!(
            eta_ms(10, 1, 100_000),
            None,
            "1 in 100 000 is startup cost, not a rate"
        );

        // Half done in ten seconds ⇒ ten seconds left.
        assert_eq!(eta_ms(10_000, 50, 100), Some(10_000));
        // A quarter done in a minute ⇒ three minutes left.
        assert_eq!(eta_ms(60_000, 25, 100), Some(180_000));
    }

    #[test]
    fn a_counted_phase_carries_its_eta_and_a_bare_phase_does_not() {
        let bare = ProgressEvent::phase("linking");
        assert_eq!(bare.current, 0);
        assert_eq!(bare.total, 0);
        assert_eq!(bare.eta_ms, None);

        let counted = ProgressEvent::counted("erasing", 50, 100, 4_000);
        assert_eq!(counted.current, 50);
        assert_eq!(counted.eta_ms, Some(4_000));
    }

    #[test]
    fn a_report_carries_the_call_it_belongs_to() {
        // The identity is the point: without it a phase from a subagent's tool and one from the
        // turn's own tool read the same, and a frontend cannot put either on the right card.
        let seen: Arc<Mutex<Vec<(String, u64, ProgressEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            Arc::new(move |tool: &str, seq: u64, event: ProgressEvent| {
                seen.lock().unwrap().push((tool.to_string(), seq, event));
            }) as ProgressSink
        };

        let reporter = ProgressReporter::new("flash", 7, sink);
        assert_eq!(reporter.tool(), "flash");
        assert_eq!(reporter.seq(), 7);
        reporter.phase("erasing");
        reporter.counted("writing", 1, 4, 500);

        let entries = seen.lock().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "flash");
        assert_eq!(entries[0].1, 7);
        assert_eq!(entries[0].2.phase, "erasing");
        assert_eq!(entries[1].2.total, 4);
        drop(entries);
        // Cloning a reporter reports to the same place: a tool that hands one to a helper does
        // not go quiet, and the clone carries the same identity.
        let clone = reporter.clone();
        assert_eq!(clone.seq(), 7);
        clone.phase("verify");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2].2.phase, "verify");
    }
}
