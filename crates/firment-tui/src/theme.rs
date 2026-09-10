//! Design tokens for the TUI, with the terminal's colour capability in mind.
//!
//! Mirrors docs/design/tokens.md. The TUI is the one surface that cannot assume
//! what it is drawing on: the same binary runs in Windows Console, in tmux, over
//! SSH to a dev board, and inside a CI pipe. So tokens here are not plain
//! colours -- they are functions of the detected [`Tier`].
//!
//! # Why most of the UI keeps NAMED colours
//!
//! ratatui's named colours (`Color::Green`, `Color::DarkGray`, …) are emitted as
//! the terminal's own palette entries. That adapts to a light terminal: it is
//! the user's theme, not ours, deciding what "green" looks like against their
//! background. Hard-coding RGB everywhere would look right on the machine this
//! was written on and wrong on a light-background terminal.
//!
//! So the split is:
//!
//! * **Semantic** states (success, failure, warning, chrome) stay named at every
//!   tier. They are already readable on both backgrounds, and they are the parts
//!   a user reads fastest.
//! * **Brand** accents (the transcript border, the input border) use the real
//!   brand green [`ACID`] when the terminal can show it, and fall back to a named
//!   colour when it cannot.
//!
//! # Degradation
//!
//! `NO_COLOR` set (any value) -> [`Tier::Mono`]: every accent collapses to the
//! terminal default, and callers lean on bold/reverse instead. `COLORTERM` of
//! `truecolor`/`24bit` -> [`Tier::TrueColor`]. Everything else -> [`Tier::Ansi`],
//! which is the pre-existing appearance and therefore the safe default.

use ratatui::style::Color;

/// Brand green, straight from the tokens. Only ever used at [`Tier::TrueColor`].
pub(crate) const ACID: (u8, u8, u8) = (0xB4, 0xF7, 0x79);
/// Success green: a different hue from the brand green on purpose.
pub(crate) const SUCCESS: (u8, u8, u8) = (0x86, 0xEF, 0xAC);
/// Failure/removed red.
pub(crate) const DANGER: (u8, u8, u8) = (0xFD, 0xA4, 0xAF);
/// Warning.
pub(crate) const WARN: (u8, u8, u8) = (0xEA, 0xB3, 0x08);
/// Secondary text.
pub(crate) const MUTED: (u8, u8, u8) = (0xA1, 0xA1, 0xAA);
/// Diff hunk headers and context.
pub(crate) const META: (u8, u8, u8) = (0xA1, 0xA1, 0xAA);

/// Added/removed row backgrounds for the diff body.
pub(crate) const DIFF_ADDED_BG: (u8, u8, u8) = (0x14, 0x31, 0x1C);
pub(crate) const DIFF_REMOVED_BG: (u8, u8, u8) = (0x3B, 0x12, 0x18);

/// How much colour the terminal can be trusted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    /// `NO_COLOR` is set: no colour at all, structure comes from attributes.
    Mono,
    /// 16 colours; named colours only, which is also the pre-existing look.
    Ansi,
    /// 24-bit colour available.
    TrueColor,
}

impl Tier {
    /// Detect from the environment.
    ///
    /// The `NO_COLOR` check comes first and is deliberately blunt (the spec is
    /// "set to any value, including empty"): a user who asked for no colour must
    /// not get brand green because `COLORTERM` happens to be set too. Windows
    /// Terminal and modern ConHost both set `COLORTERM` or `WT_SESSION`; a bare
    /// `TERM=xterm` does not promise 24-bit, so it stays at [`Tier::Ansi`].
    pub(crate) fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Tier::Mono;
        }
        let truecolor = std::env::var("COLORTERM")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("truecolor") || v.contains("24bit")
            })
            .unwrap_or(false);
        if truecolor {
            return Tier::TrueColor;
        }
        // Apple's Terminal advertises 24-bit through TERM rather than COLORTERM.
        if std::env::var("TERM").is_ok_and(|t| t.contains("truecolor") || t.contains("24bit")) {
            return Tier::TrueColor;
        }
        Tier::Ansi
    }

    /// A token colour at this tier, or `None` under `NO_COLOR`.
    fn rgb(self, (r, g, b): (u8, u8, u8)) -> Option<Color> {
        match self {
            Tier::Mono => None,
            Tier::Ansi => None, // falls back to the named variant at the call site
            Tier::TrueColor => Some(Color::Rgb(r, g, b)),
        }
    }
}

/// The transcript and input border: the brand accent.
///
/// This is the one place the TUI shows brand colour, and it is the right one --
/// the frame around the agent's output is the identity anchor, the same way the
/// mark is in the GUI. At lower tiers it keeps the cyan it has always been, so
/// nothing about an existing session changes.
pub(crate) fn accent(tier: Tier) -> Color {
    tier.rgb(ACID).unwrap_or(Color::Cyan)
}

/// Text that sits ON the accent (none today, reserved for filled accents).
pub(crate) fn on_accent(tier: Tier) -> Color {
    tier.rgb((0x15, 0x20, 0x0D)).unwrap_or(Color::Black)
}

/// Success: a passed check, a finished tool.
pub(crate) fn success(tier: Tier) -> Color {
    tier.rgb(SUCCESS).unwrap_or(Color::Green)
}

/// Failure. Reserved for real errors -- an unknown state is not a failure.
pub(crate) fn danger(tier: Tier) -> Color {
    tier.rgb(DANGER).unwrap_or(Color::Red)
}

/// Warning: degraded but working.
pub(crate) fn warn(tier: Tier) -> Color {
    tier.rgb(WARN).unwrap_or(Color::Yellow)
}

/// Secondary text and chrome.
pub(crate) fn muted(tier: Tier) -> Color {
    tier.rgb(MUTED).unwrap_or(Color::DarkGray)
}

/// Diff metadata: hunk headers, context lines.
pub(crate) fn meta(tier: Tier) -> Color {
    tier.rgb(META).unwrap_or(Color::DarkGray)
}

/// Background of an added diff row. `None` at Ansi/Mono: a filled background is
/// the first thing that turns into an unreadable block when the palette is
/// unknown, so those tiers keep the `+` marker and the foreground colour only.
pub(crate) fn diff_added_bg(tier: Tier) -> Option<Color> {
    tier.rgb(DIFF_ADDED_BG)
}

/// Background of a removed diff row. See [`diff_added_bg`].
pub(crate) fn diff_removed_bg(tier: Tier) -> Option<Color> {
    tier.rgb(DIFF_REMOVED_BG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_degrade_in_the_expected_order() {
        // Mono has no colour at all, so every accessor falls back to the named
        // variant rather than returning an RGB value the terminal cannot show.
        assert_eq!(accent(Tier::Mono), Color::Cyan);
        assert_eq!(success(Tier::Mono), Color::Green);
        assert_eq!(danger(Tier::Mono), Color::Red);
        assert_eq!(warn(Tier::Mono), Color::Yellow);
        assert_eq!(muted(Tier::Mono), Color::DarkGray);
        assert_eq!(diff_added_bg(Tier::Mono), None);

        // Ansi is the pre-existing appearance: named colours, no filled diff.
        assert_eq!(accent(Tier::Ansi), Color::Cyan);
        assert_eq!(diff_removed_bg(Tier::Ansi), None);

        // TrueColor gets the real tokens.
        assert_eq!(accent(Tier::TrueColor), Color::Rgb(0xB4, 0xF7, 0x79));
        assert_eq!(success(Tier::TrueColor), Color::Rgb(0x86, 0xEF, 0xAC));
        assert_eq!(
            diff_added_bg(Tier::TrueColor),
            Some(Color::Rgb(0x14, 0x31, 0x1C))
        );
    }

    #[test]
    fn a_filled_background_is_truecolor_only() {
        // The rule that matters: at 16 colours a hard-coded background is a
        // coin flip against an unknown palette, so it is simply not emitted.
        for tier in [Tier::Mono, Tier::Ansi] {
            assert_eq!(diff_added_bg(tier), None, "{tier:?}");
            assert_eq!(diff_removed_bg(tier), None, "{tier:?}");
        }
        assert!(diff_added_bg(Tier::TrueColor).is_some());
    }
}
