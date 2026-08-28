//! Incremental UTF-8 line splitting for byte streams that arrive in
//! arbitrary chunks.
//!
//! A serial port (or any `Read`) hands back whatever bytes happen to be in
//! the driver buffer at that instant — the chunk boundaries have nothing to
//! do with character boundaries. Decoding each chunk on its own therefore
//! turns any multi-byte character that straddles a boundary into U+FFFD: a
//! 3-byte CJK glyph split as `[E4] | [B8 AD]` decodes to two replacement
//! characters, not one Chinese character.
//!
//! `LineSplitter` accumulates raw bytes and only decodes when it has a
//! complete line, which is why the boundary problem disappears:
//!
//! * **Inside a line** — UTF-8 is self-synchronising, so `0x0A` can never
//!   occur within a multi-byte sequence. Scanning for `\n` at the byte level
//!   is therefore exact, and a fully received line decodes cleanly no matter
//!   how it was chopped up.
//! * **At the tail** — the bytes after the last `\n` are a partial line. For
//!   callers that only output on newline (CLI, monitor, HIL) this is
//!   harmless; they flush the remainder with [`take_tail`] when the read
//!   ends. For the GUI monitor, which must also render devices that never
//!   send newlines, [`take_flushable`] emits everything up to the last
//!   complete character and keeps the incomplete tail for the next chunk.

/// Upper bound on how many bytes are buffered for a single line.
///
/// A device streaming forever without a newline (baud rate mismatched, or a
/// firmware loop that prints without `\n`) would otherwise grow the buffer
/// without limit. Bytes past this cap are dropped until the next `\n`.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Accumulates a byte stream and hands back decoded lines.
pub struct LineSplitter {
    /// Bytes received but not yet emitted: everything after the last `\n`.
    buf: Vec<u8>,
    max_line_bytes: usize,
}

impl LineSplitter {
    pub fn new(max_line_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_line_bytes,
        }
    }

    /// Append `bytes` and invoke `on_line` once per complete line.
    ///
    /// The line passed to the callback excludes the terminating `\n` and a
    /// single trailing `\r` (so CRLF input yields clean lines), but an
    /// interior `\r` is preserved — devices use it to overwrite the current
    /// line for progress output.
    pub fn feed<F: FnMut(&str)>(&mut self, bytes: &[u8], on_line: &mut F) {
        self.buf.extend_from_slice(bytes);
        let mut start = 0usize;
        while let Some(rel) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let end = start + rel;
            let mut line = &self.buf[start..end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.len() > self.max_line_bytes {
                line = &line[..self.max_line_bytes];
            }
            let text = String::from_utf8_lossy(line);
            on_line(text.as_ref());
            start = end + 1;
        }
        self.buf.drain(..start);
        // No newline yet: keep only the first `max_line_bytes` so an
        // endless newline-less stream cannot grow the buffer without bound.
        if self.buf.len() > self.max_line_bytes {
            self.buf.truncate(self.max_line_bytes);
        }
    }

    /// Take everything that is safe to display right now: all complete
    /// characters, stopping before an incomplete trailing sequence.
    ///
    /// Bytes that are genuinely invalid (not merely truncated) are replaced
    /// with U+FFFD and consumed — the same observable behaviour as
    /// [`String::from_utf8_lossy`], so a corrupt device stream stays visible
    /// instead of being silently dropped. Consuming them is what keeps
    /// `valid_up_to()` from pinning at zero and wedging the buffer head.
    ///
    /// At most 3 bytes can be held back (UTF-8 is 4 bytes at the longest),
    /// so the added latency is half a character — imperceptible even at
    /// 9600 baud.
    pub fn take_flushable(&mut self) -> String {
        let mut out = String::new();
        let mut consumed = 0usize;
        loop {
            let rest = &self.buf[consumed..];
            if rest.is_empty() {
                break;
            }
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    consumed += rest.len();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    if valid > 0 {
                        out.push_str(String::from_utf8_lossy(&rest[..valid]).as_ref());
                        consumed += valid;
                    }
                    match e.error_len() {
                        // Genuinely invalid: replace and step over it.
                        Some(n) => {
                            out.push('\u{FFFD}');
                            consumed += n;
                        }
                        // Truncated, not invalid: wait for the next chunk.
                        None => break,
                    }
                }
            }
        }
        self.buf.drain(..consumed);
        out
    }

    /// Flush whatever partial line is left, for use when the stream ends
    /// (timeout, cancellation, port closed). Returns `None` if empty.
    pub fn take_tail(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(text)
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect lines produced by feeding `chunks` in order.
    fn feed_all(chunks: &[&[u8]]) -> Vec<String> {
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut |line| out.push(line.to_string()));
        }
        out
    }

    // -- the actual defect: a CJK character split across chunks -----------

    #[test]
    fn cjk_split_after_one_byte() {
        assert_eq!(feed_all(&[b"ab\xe4", b"\xb8\xad\n"]), vec!["ab中"]);
    }

    #[test]
    fn cjk_split_after_two_bytes() {
        assert_eq!(feed_all(&[b"\xe4\xb8", b"\xad\n"]), vec!["中"]);
    }

    #[test]
    fn cjk_unsplit_in_one_chunk() {
        assert_eq!(
            feed_all(&[b"hello \xe4\xb8\xad\xe6\x96\x87\n"]),
            vec!["hello 中文"]
        );
    }

    #[test]
    fn no_line_without_newline() {
        assert!(feed_all(&[b"abc"]).is_empty());
    }

    // -- carriage returns -------------------------------------------------

    #[test]
    fn strips_single_trailing_cr() {
        assert_eq!(feed_all(&[b"a\r\n"]), vec!["a"]);
    }

    #[test]
    fn keeps_second_cr() {
        assert_eq!(feed_all(&[b"a\r\r\n"]), vec!["a\r"]);
    }

    #[test]
    fn keeps_inline_cr() {
        assert_eq!(feed_all(&[b"a\rb\n"]), vec!["a\rb"]);
    }

    // -- invalid bytes ----------------------------------------------------

    #[test]
    fn invalid_0xff_becomes_replacement() {
        assert_eq!(feed_all(&[b"a\xffb\n"]), vec!["a\u{FFFD}b"]);
    }

    #[test]
    fn truncated_then_invalid() {
        // 0xE4 starts a 3-byte sequence but 0xFF is not a continuation
        // byte, so both bytes are invalid — two replacement characters.
        assert_eq!(feed_all(&[b"\xe4\xff\n"]), vec!["\u{FFFD}\u{FFFD}"]);
    }

    // -- take_flushable (GUI path) ----------------------------------------

    #[test]
    fn emoji_byte_by_byte() {
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut noop = |_: &str| {};
        // A 4-byte emoji arriving one byte per chunk must not emit a
        // replacement character at any intermediate step.
        s.feed(&[0xf0], &mut noop);
        assert_eq!(s.take_flushable(), "");
        s.feed(&[0x9f], &mut noop);
        assert_eq!(s.take_flushable(), "");
        s.feed(&[0x98], &mut noop);
        assert_eq!(s.take_flushable(), "");
        s.feed(&[0x80], &mut noop);
        assert_eq!(s.take_flushable(), "😀");
        assert!(s.is_empty());
    }

    #[test]
    fn flushable_keeps_truncated_tail() {
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut noop = |_: &str| {};
        s.feed(b"hi\xe4\xb8", &mut noop);
        assert_eq!(s.take_flushable(), "hi");
        assert!(!s.is_empty());
    }

    #[test]
    fn flushable_advances_past_invalid() {
        // Regression guard: an invalid byte at the buffer head must be
        // consumed, otherwise valid_up_to() stays 0 forever and the
        // splitter wedges, never emitting the good bytes behind it.
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut noop = |_: &str| {};
        let mut bytes = vec![0xff];
        bytes.extend_from_slice("中".as_bytes());
        s.feed(&bytes, &mut noop);
        assert_eq!(s.take_flushable(), "\u{FFFD}中");
        assert_eq!(s.take_flushable(), "");
    }

    // -- bounds -----------------------------------------------------------

    #[test]
    fn oversize_line_is_bounded() {
        let mut s = LineSplitter::new(8);
        let mut out = Vec::new();
        s.feed(&[b'x'; 100], &mut |line| out.push(line.to_string()));
        assert!(s.buf.len() <= 8, "buffer grew to {}", s.buf.len());
        s.feed(b"\n", &mut |line| out.push(line.to_string()));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 8);
    }

    // -- teardown ---------------------------------------------------------

    #[test]
    fn take_tail_flushes_residual() {
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut noop = |_: &str| {};
        s.feed(b"tail", &mut noop);
        assert_eq!(s.take_tail(), Some("tail".to_string()));
        assert!(s.is_empty());
        assert_eq!(s.take_tail(), None);
    }

    #[test]
    fn empty_feed_is_noop() {
        let mut s = LineSplitter::new(MAX_LINE_BYTES);
        let mut out = Vec::new();
        s.feed(b"", &mut |line| out.push(line.to_string()));
        assert!(out.is_empty());
        assert!(s.is_empty());
    }

    // -- end-to-end regressions -------------------------------------------

    /// The strongest guard against this class of bug: feed the same text
    /// split at *every* possible offset and require identical output.
    #[test]
    fn all_split_points_roundtrip() {
        let text = "传感器数据: 温度 25.6°C 正常";
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\n');
        for i in 0..=bytes.len() {
            let out = feed_all(&[&bytes[..i], &bytes[i..]]);
            assert_eq!(out.len(), 1, "split at {i} produced {out:?}");
            assert_eq!(out[0], text, "split at {i} garbled the line");
            assert!(!out[0].contains('\u{FFFD}'), "split at {i} lost a byte");
        }
    }

    /// The read buffer is 4096 bytes, so a character landing on that
    /// boundary is the shape this bug took in practice.
    #[test]
    fn four_k_boundary_regression() {
        let mut first = vec![b'a'; 4095];
        first.extend_from_slice(&[0xe4]); // first byte of 中
        let mut second = vec![0xb8, 0xad]; // rest of 中
        second.push(b'\n');
        let out = feed_all(&[&first, &second]);
        assert_eq!(out.len(), 1);
        assert!(!out[0].contains('\u{FFFD}'));
        assert!(out[0].ends_with('中'));
        // 4095 ASCII + one 3-byte character.
        assert_eq!(out[0].chars().count(), 4096);
        assert_eq!(out[0].len(), 4098);
    }
}
