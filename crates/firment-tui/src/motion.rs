//! How often the TUI is allowed to repaint, and whether it animates at all.
//!
//! Two separate concerns that used to be one number. The event loop ticks every
//! 25ms so that a paste lands promptly, but that tick also drove the spinner's
//! frames, so the UI repainted at 40fps. On a local, modern terminal that is
//! fine. It is not fine over `ssh` to a dev board, through `tmux`, or in
//! Windows Console, where every full-screen repaint is a real cost and a
//! spinner that moves smoothly locally makes the whole session crawl remotely.
//!
//! So: **animation is capped at 15fps**, and it is switched off entirely where
//! it cannot pay for itself. The cap only applies to repaints whose whole
//! purpose is animation -- a repaint caused by real output is never delayed,
//! because a lagging transcript is worse than a jumpy spinner.
//!
//! The policy is a plain value decided once at startup from the environment,
//! with an explicit flag as the final word. It is deliberately not re-evaluated
//! per frame: the answer cannot change mid-session, and asking `std::env` in a
//! hot loop is how a "temporary" check becomes permanent.

use std::io::IsTerminal;
use std::time::{Duration, Instant};

/// The animation frame budget: 15fps.
///
/// Not 60, and not 30. `firm` over SSH to a board is a normal way to work, and
/// the value of a smoothly rotating spinner does not survive a 200ms round
/// trip. 15fps still reads as motion rather than as a still image.
pub const ANIM_PERIOD: Duration = Duration::from_millis(66);

/// Whether the TUI may animate, decided once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Motion {
    enabled: bool,
}

impl Motion {
    /// The policy for a known environment. Pure, so the rules are testable.
    ///
    /// Every disjunct is a place where animation has been measured to cost more
    /// than it returns. `no_anim` is checked first and wins outright: a user who
    /// typed the flag has already made the decision.
    pub fn new(no_anim: bool, ssh: bool, dumb_term: bool, tty: bool) -> Self {
        Self {
            enabled: !no_anim && tty && !ssh && !dumb_term,
        }
    }

    /// The policy for this process.
    pub fn from_env(no_anim: bool) -> Self {
        // `SSH_CONNECTION`/`SSH_TTY` are what ssh actually sets; `TERM` is the
        // capability signal. Both are read here rather than in the loop.
        let ssh =
            std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
        let dumb_term = std::env::var("TERM").is_ok_and(|t| t == "dumb");
        let tty = std::io::stdout().is_terminal();
        Self::new(no_anim, ssh, dumb_term, tty)
    }

    /// Whether animation repaints are allowed at all.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether an animation repaint is due, given when the last one happened.
    ///
    /// `None` means nothing has been drawn yet, which is always due. A disabled
    /// policy is never due, so the caller can ask unconditionally.
    pub fn repaint_due(&self, last_draw: Option<Instant>, now: Instant) -> bool {
        self.enabled && last_draw.is_none_or(|t| now.duration_since(t) >= ANIM_PERIOD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Motion {
        Motion::new(false, false, false, true)
    }

    #[test]
    fn animates_on_a_local_capable_terminal() {
        assert!(on().enabled());
    }

    #[test]
    fn the_explicit_flag_wins_over_everything() {
        // A user who typed --no-anim has made the decision; do not out-vote it
        // with a capable terminal.
        assert!(!Motion::new(true, false, false, true).enabled());
    }

    #[test]
    fn does_not_animate_over_ssh() {
        // The whole point of the cap: a full-screen repaint over a link is a
        // real cost, and `firm` over ssh to a board is a normal way to work.
        assert!(!Motion::new(false, true, false, true).enabled());
    }

    #[test]
    fn does_not_animate_on_a_dumb_terminal() {
        assert!(!Motion::new(false, false, true, true).enabled());
    }

    #[test]
    fn does_not_animate_without_a_terminal() {
        // Nothing to animate onto.
        assert!(!Motion::new(false, false, false, false).enabled());
    }

    #[test]
    fn the_period_is_fifteen_frames_a_second() {
        assert_eq!(ANIM_PERIOD, Duration::from_millis(66));
        // Stated as the property rather than the number: 66ms is 15fps, and a
        // "sensible" 16ms here would silently be 60fps again.
        let fps = 1000.0 / ANIM_PERIOD.as_millis() as f64;
        assert!((fps - 15.0).abs() < 0.5, "expected ~15fps, got {fps}");
    }

    #[test]
    fn a_frame_is_due_once_the_period_has_passed() {
        let now = Instant::now();
        assert!(on().repaint_due(None, now), "the first frame is always due");
        assert!(!on().repaint_due(Some(now - Duration::from_millis(65)), now));
        assert!(on().repaint_due(Some(now - ANIM_PERIOD), now));
        assert!(on().repaint_due(Some(now - Duration::from_millis(500)), now));
    }

    #[test]
    fn nothing_is_ever_due_when_animation_is_off() {
        let off = Motion::new(true, false, false, true);
        let now = Instant::now();
        assert!(!off.repaint_due(None, now));
        assert!(!off.repaint_due(Some(now - Duration::from_secs(10)), now));
    }
}
