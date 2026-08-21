//! Multi-line bracketed-paste handling: bursts of rapid key events (e.g. a
//! terminal that fakes paste by replaying keystrokes) are collapsed into a
//! single input insertion so the UI does not lag or mangle the text.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
pub(crate) struct PasteBlock {
    pub(crate) placeholder: String,
    pub(crate) text: String,
}

/// Paste-burst detection.
///
/// When bracketed paste is not enabled, Windows terminals inject pasted text
/// as a rapid stream of keystrokes (often ending with Enter). `PasteBurst`
/// recognizes plain-text keys arriving within 35ms as one paste: Enter counts
/// as a newline instead of submit during the burst, and the whole buffer is
/// collapsed into one paste after it goes quiet.
#[derive(Debug, Default)]
pub(crate) struct PasteBurst {
    /// Arrival time of the previous plain-text char, used to detect a burst.
    pub(crate) last_char_time: Option<Instant>,
    /// Most recent char inserted directly into the input, with its position
    /// (used for retro-capture).
    pub(crate) last_inserted: Option<(usize, char)>,
    /// First ASCII char being held while waiting for a second char to confirm
    /// a burst.
    pub(crate) held: Option<(char, Instant)>,
    /// Confirmed paste buffer.
    pub(crate) buffer: Option<String>,
    /// Last write time of the buffer.
    pub(crate) buffer_last_update: Option<Instant>,
    /// Enters arriving before this time are treated as newlines (protection
    /// window after a burst flushes).
    pub(crate) suppress_enter_until: Option<Instant>,
    /// Outputs waiting to be applied by the App.
    pub(crate) out: VecDeque<PasteOut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PasteOut {
    /// Insert into the input as a normal char.
    InsertChar(char),
    /// Remove the char at this position (reclaim the retro-captured prefix).
    RemoveAt(usize, char),
    /// Insert as one paste (auto-collapsed).
    HandlePaste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnterAction {
    Submit,
    Newline,
    BufferNewline,
}

impl PasteBurst {
    const BURST_INTERVAL: Duration = Duration::from_millis(35);
    const HOLD_DELAY: Duration = Duration::from_millis(30);
    const FLUSH_DELAY: Duration = Duration::from_millis(80);
    const SUPPRESS_WINDOW: Duration = Duration::from_millis(120);

    /// Handle a plain char with no modifiers. `cursor` is where it will be
    /// inserted.
    pub(crate) fn on_plain_char(&mut self, c: char, now: Instant, cursor: usize) {
        let prev_time = self.last_char_time;
        self.last_char_time = Some(now);

        // Burst confirmed: keep appending to the buffer.
        if let Some(buf) = &mut self.buffer {
            buf.push(c);
            self.buffer_last_update = Some(now);
            return;
        }

        // A first char is held: a second char arriving quickly confirms a burst.
        if let Some((held, at)) = self.held.take() {
            if now.duration_since(at) <= Self::HOLD_DELAY {
                let mut buf = String::with_capacity(2);
                buf.push(held);
                buf.push(c);
                self.buffer = Some(buf);
                self.buffer_last_update = Some(now);
                self.last_inserted = None;
                return;
            }
            // Hold timed out: emit the old char as normal input.
            self.out.push_back(PasteOut::InsertChar(held));
        }

        // Retro-capture: a non-ASCII first char was inserted immediately; when
        // a second char arrives quickly, reclaim that prefix into the buffer
        // (so pasted CJK text does not leave a stray first char).
        if let (Some(at), Some((pos, prev))) = (prev_time, self.last_inserted)
            && now.duration_since(at) <= Self::BURST_INTERVAL
        {
            let mut buf = String::with_capacity(2);
            buf.push(prev);
            buf.push(c);
            self.buffer = Some(buf);
            self.buffer_last_update = Some(now);
            self.last_inserted = None;
            self.out.push_back(PasteOut::RemoveAt(pos, prev));
            return;
        }

        if c.is_ascii() {
            // Hold ASCII briefly: detects bursts without flicker on single keys.
            self.held = Some((c, now));
            self.last_inserted = None;
        } else {
            // Non-ASCII (IME/CJK) is not held; insert immediately and record
            // the position.
            self.last_inserted = Some((cursor, c));
            self.out.push_back(PasteOut::InsertChar(c));
        }
    }

    /// Enter: returns Newline while a burst is active or within the protection
    /// window, otherwise Submit.
    pub(crate) fn on_enter(&mut self, now: Instant) -> EnterAction {
        if let Some((held, _)) = self.held.take() {
            // The first char is still held: release it into the input and treat
            // Enter as a newline, so a trailing Enter in a single-char paste
            // cannot submit.
            self.out.push_back(PasteOut::InsertChar(held));
            return EnterAction::Newline;
        }
        if self.buffer.is_some() || self.suppress_enter_until.is_some_and(|t| now <= t) {
            if let Some(buf) = &mut self.buffer {
                buf.push('\n');
                self.buffer_last_update = Some(now);
                EnterAction::BufferNewline
            } else {
                EnterAction::Newline
            }
        } else {
            EnterAction::Submit
        }
    }

    /// Shift+Enter: merged into the buffer during a burst, otherwise normal
    /// newline behavior.
    pub(crate) fn on_shift_enter(&mut self, now: Instant) -> EnterAction {
        if let Some(buf) = &mut self.buffer {
            buf.push('\n');
            self.buffer_last_update = Some(now);
            EnterAction::BufferNewline
        } else {
            EnterAction::Newline
        }
    }

    /// Due-time handling: a held char past its timeout is emitted as normal
    /// input; a buffer idle past its timeout is flushed as one paste.
    pub(crate) fn flush_if_due(&mut self, now: Instant) {
        if let Some((c, at)) = self.held
            && now.duration_since(at) >= Self::HOLD_DELAY
        {
            self.held = None;
            self.out.push_back(PasteOut::InsertChar(c));
        }
        let ready = self
            .buffer_last_update
            .is_some_and(|at| now.duration_since(at) >= Self::FLUSH_DELAY);
        if ready && let Some(text) = self.buffer.take() {
            self.buffer_last_update = None;
            self.last_inserted = None;
            self.out.push_back(PasteOut::HandlePaste(text));
            self.suppress_enter_until = Some(now + Self::SUPPRESS_WINDOW);
        }
    }

    /// Clear burst state; a held char must not be lost, so queue it as output
    /// first.
    pub(crate) fn clear(&mut self) {
        if let Some((c, _)) = self.held.take() {
            self.out.push_back(PasteOut::InsertChar(c));
        }
        self.last_char_time = None;
        self.last_inserted = None;
        self.buffer = None;
        self.buffer_last_update = None;
        self.suppress_enter_until = None;
    }

    pub(crate) fn drain_outputs(&mut self) -> Vec<PasteOut> {
        self.out.drain(..).collect()
    }
}
