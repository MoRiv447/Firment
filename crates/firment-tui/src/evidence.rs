//! The verification ladder, as a session's progress up it.
//!
//! The ladder itself is not defined here. `firment_tools::tools::ladder_rung` is
//! what the `hil` tool advances, and it is the authority on which tool reaches
//! which rung; this module only keeps track of what a session has reached, so
//! that the EVIDENCE panel and the tools can never disagree about what was
//! proven. The one thing written down here is the *labels*, and a test pins them
//! to the canonical mapping.
//!
//! Rungs are tracked individually rather than as a high-water mark. The system
//! prompt is explicit that a higher level never implies the ones below it
//! succeeded for the user's goal, so a ladder that fills everything below its
//! highest rung would be making exactly the claim the prompt forbids.

use firment_tools::tools::ladder_rung;

/// The five rungs, in ascending order, with the names they are shown under.
///
/// Rung 1 has no tool behind it -- "the code exists" is the floor every session
/// starts from -- so its label has no canonical source to be read from. The
/// other four do, and `labels_match_the_tool_ladder` fails if they drift.
pub const RUNGS: [(u8, &str); 5] = [
    (1, "code"),
    (2, "build"),
    (3, "deploy"),
    (4, "runtime"),
    (5, "physical"),
];

/// What the panel shows for one rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RungState {
    /// A step at this rung succeeded.
    Proven,
    /// A step at this rung is running right now.
    Running,
    /// Nothing has reached it.
    Untouched,
}

/// A session's progress up the ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// Indexed by rung-1: whether a step at that rung has succeeded.
    reached: [bool; RUNGS.len()],
    /// The rung of the step currently running, if any.
    running: Option<u8>,
    /// Whether any edit has been made. Rung 1 is not "a tool ran", it is "there
    /// is code", so it is earned by writing a file rather than by a build.
    code_written: bool,
}

impl Default for Evidence {
    fn default() -> Self {
        Self {
            reached: [false; RUNGS.len()],
            running: None,
            code_written: false,
        }
    }
}

impl Evidence {
    /// A step of this kind has started.
    pub fn begin(&mut self, kind: &str) {
        // An edit is not a ladder step with a rung of its own, and it must not
        // earn rung 1 on the attempt: a write that fails leaves no code behind.
        if is_edit(kind) {
            return;
        }
        if let Some((rung, _)) = ladder_rung(kind) {
            self.running = Some(rung);
        }
    }

    /// A step of this kind has finished.
    ///
    /// Only success advances the ladder: a build that failed proves nothing
    /// about the build rung, which is the whole reason this is not a counter of
    /// tools that ran.
    pub fn finish(&mut self, kind: &str, ok: bool) {
        if is_edit(kind) {
            self.code_written = self.code_written || ok;
            return;
        }
        let Some((rung, _)) = ladder_rung(kind) else {
            return;
        };
        if self.running == Some(rung) {
            self.running = None;
        }
        if ok && let Some(slot) = self.reached.get_mut(rung as usize - 1) {
            *slot = true;
        }
    }

    /// The state to draw for one rung.
    pub fn state(&self, rung: u8) -> RungState {
        if rung == 1 {
            // Every session with code on the board has, trivially, code.
            return if self.code_written {
                RungState::Proven
            } else {
                RungState::Untouched
            };
        }
        if self
            .reached
            .get(rung as usize - 1)
            .copied()
            .unwrap_or(false)
        {
            return RungState::Proven;
        }
        if self.running == Some(rung) {
            return RungState::Running;
        }
        RungState::Untouched
    }

    /// The rungs to draw, in order. The panel shows all of them, always: a
    /// ladder that appears one rung at a time cannot be read as a ladder.
    pub fn rows(&self) -> impl Iterator<Item = (u8, &'static str, RungState)> + '_ {
        RUNGS
            .iter()
            .map(move |&(rung, label)| (rung, label, self.state(rung)))
    }

    /// The highest rung proven, if any. This is what a completion claim is
    /// measured against.
    pub fn highest(&self) -> Option<u8> {
        RUNGS
            .iter()
            .filter(|&&(rung, _)| self.state(rung) == RungState::Proven)
            .map(|&(rung, _)| rung)
            .max()
    }
}

/// Whether a tool writes code, which is what rung 1 is about.
fn is_edit(kind: &str) -> bool {
    matches!(kind, "edit_file" | "write_file" | "write" | "edit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_tool_ladder() {
        // The point of this test: the panel's labels are written down here, and
        // the rungs they belong to are defined in the tools crate. If that
        // mapping is ever renamed, this fails instead of the panel quietly
        // showing a name the tools no longer use.
        for (kind, rung) in [
            ("build", 2u8),
            ("flash", 3),
            ("run", 4),
            ("monitor", 4),
            ("trace", 4),
            ("observe", 5),
            ("la", 5),
        ] {
            let (got_rung, got_label) = ladder_rung(kind).expect("canonical kind");
            assert_eq!(got_rung, rung, "rung for {kind}");
            assert_eq!(
                got_label,
                RUNGS[rung as usize - 1].1,
                "label for {kind} drifted from RUNGS"
            );
        }
    }

    #[test]
    fn nothing_is_proven_before_anything_ran() {
        let e = Evidence::default();
        assert_eq!(e.highest(), None);
        assert!(e.rows().all(|(_, _, s)| s == RungState::Untouched));
    }

    #[test]
    fn writing_code_earns_the_first_rung_and_no_more() {
        let mut e = Evidence::default();
        e.begin("edit_file");
        // Starting an edit earns nothing: a write that fails leaves no code.
        assert_eq!(e.state(1), RungState::Untouched);
        e.finish("edit_file", true);
        assert_eq!(e.state(1), RungState::Proven);
        // An edit is rung 1, not a build: nothing has compiled yet.
        assert_eq!(e.state(2), RungState::Untouched);
        assert_eq!(e.highest(), Some(1));
    }

    #[test]
    fn a_failed_edit_leaves_the_first_rung_untried() {
        let mut e = Evidence::default();
        e.begin("edit_file");
        e.finish("edit_file", false);
        assert_eq!(e.state(1), RungState::Untouched);
        assert_eq!(e.highest(), None);
    }

    #[test]
    fn a_successful_step_advances_its_rung() {
        let mut e = Evidence::default();
        e.finish("build", true);
        assert_eq!(e.state(2), RungState::Proven);
        assert_eq!(e.highest(), Some(2));
    }

    #[test]
    fn a_failed_step_advances_nothing() {
        let mut e = Evidence::default();
        e.finish("build", false);
        assert_eq!(e.state(2), RungState::Untouched);
        assert_eq!(e.highest(), None);
    }

    #[test]
    fn a_running_step_shows_as_running_not_as_proven() {
        let mut e = Evidence::default();
        e.begin("flash");
        assert_eq!(e.state(3), RungState::Running);
        assert_eq!(e.highest(), None, "a step in flight has proven nothing yet");
        // ...and it becomes proven only when it reports back.
        e.finish("flash", true);
        assert_eq!(e.state(3), RungState::Proven);
        assert_eq!(e.highest(), Some(3));
    }

    #[test]
    fn rungs_are_tracked_individually_not_as_a_high_water_mark() {
        // The prompt is explicit that a higher rung never implies the ones below
        // it succeeded for the user's goal. Here the device was deployed without
        // a build step ever running, and the ladder must not claim one did.
        let mut e = Evidence::default();
        e.finish("flash", true);
        assert_eq!(e.state(3), RungState::Proven);
        assert_eq!(
            e.state(2),
            RungState::Untouched,
            "no build was ever observed"
        );
    }

    #[test]
    fn an_unknown_tool_is_not_a_rung() {
        // `elf_analyze` and `delay` are hil steps that do not advance the
        // ladder; so is anything the agent invents.
        let mut e = Evidence::default();
        e.begin("elf_analyze");
        e.finish("elf_analyze", true);
        e.finish("read_file", true);
        assert_eq!(e.highest(), None);
        assert!(e.rows().all(|(_, _, s)| s == RungState::Untouched));
    }

    #[test]
    fn the_running_step_clears_when_it_reports() {
        let mut e = Evidence::default();
        e.begin("observe");
        e.finish("observe", false);
        // A failed observe must not leave the ladder showing a spinner forever.
        assert_eq!(e.state(5), RungState::Untouched);
        assert!(e.rows().all(|(_, _, s)| s != RungState::Running));
    }

    #[test]
    fn the_panel_always_shows_five_rungs() {
        // A ladder that grows one rung at a time cannot be read as a ladder.
        let e = Evidence::default();
        assert_eq!(e.rows().count(), 5);
        assert_eq!(
            e.rows().map(|(r, _, _)| r).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
    }
}
