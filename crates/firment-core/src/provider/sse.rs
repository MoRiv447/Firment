/// The single ceiling on how many bytes one SSE line may accumulate before the
/// stream is refused, and the tail-flush rule that goes with it.
///
/// Two numbers used to be implicit here and both were wrong in the same
/// direction. There was no ceiling at all: `line_buf` grew until the process
/// ran out of memory, while the inactivity timer re-armed on every network
/// chunk, so a slow-trickling or deliberately hostile endpoint was never
/// interrupted. And there was no tail flush: bytes were parsed only when a
/// `\n` was seen, so a gateway that closes the body without a trailing newline
/// lost its last frame -- routinely the tool-call arguments or the stop reason
/// -- and the loop reported `EndTurn` as though the reply had been complete.
///
/// The ceiling is the same number the serial capture, the HIL monitor step and
/// the subprocess drains already stop at, so one tuning of it reaches every
/// stream the agent reads. It lives here rather than in `firment-tools` because
/// a provider must not depend on a tool crate.
pub const CAPTURE_CAP_BYTES: usize = 8 * 1024 * 1024;

use crate::provider::ProviderError;

/// Accumulates a byte stream into complete lines.
///
/// [`SseLineBuffer::push`] returns only whole lines and keeps the remainder;
/// [`SseLineBuffer::finish`] hands back what is left when the stream ends
/// cleanly, as a line. That pair is the whole point: a caller that forgot
/// `finish` is the defect this type exists to make impossible, and both
/// providers route the bytes it returns through the same per-line handler as a
/// newline-terminated frame, so a tail frame is not a second code path that can
/// drift from the first.
#[derive(Default)]
pub(crate) struct SseLineBuffer {
    pending: Vec<u8>,
}

impl SseLineBuffer {
    /// Whole lines contained in `chunk`, in order. The newline itself is
    /// dropped; a `\r\n` line therefore arrives with a trailing `\r`, which
    /// every caller trims, exactly as the byte-at-a-time loop it replaced did.
    ///
    /// `Err` once an unterminated line reaches the ceiling. It is an error
    /// rather than a truncation: a partial SSE frame is not a frame, and
    /// handing the agent half a tool payload while the other half was thrown
    /// away is the failure this file refuses to commit elsewhere.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ProviderError> {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        for &b in chunk {
            if self.pending.len() >= CAPTURE_CAP_BYTES {
                return Err(ProviderError::InvalidResponse(format!(
                    "SSE line exceeded the {}-byte capture ceiling without a newline; \
                     the stream is malformed or hostile, and a partial frame is not a frame",
                    CAPTURE_CAP_BYTES
                )));
            }
            if b == b'\n' {
                lines.push(std::mem::take(&mut self.pending));
            } else {
                self.pending.push(b);
            }
        }
        Ok(lines)
    }

    /// What is buffered when the stream is over, as the final line -- empty
    /// when the body ended on a newline, which is the well-formed case.
    pub(crate) fn finish(&mut self) -> Vec<Vec<u8>> {
        if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![std::mem::take(&mut self.pending)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_without_a_newline_is_a_line_and_not_a_loss() {
        let mut buf = SseLineBuffer::default();
        assert!(buf.push(b"data: {\"partial\":true").unwrap().is_empty());
        let tail = buf.finish();
        assert_eq!(tail.len(), 1, "the unflushed bytes must arrive");
        assert_eq!(tail[0], b"data: {\"partial\":true".to_vec());
        assert!(buf.finish().is_empty(), "and only once");
    }

    #[test]
    fn a_body_ending_on_a_newline_has_no_tail() {
        let mut buf = SseLineBuffer::default();
        assert_eq!(buf.push(b"a\nb\n").unwrap().len(), 2);
        assert!(buf.finish().is_empty());
    }

    #[test]
    fn a_line_split_across_chunks_is_still_one_line() {
        let mut buf = SseLineBuffer::default();
        assert!(buf.push(b"data: {\"arg").unwrap().is_empty());
        let lines = buf.push(b"uments\":{}}\n").unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], b"data: {\"arguments\":{}}".to_vec());
    }

    #[test]
    fn crlf_lines_lose_the_cr_only_at_the_end() {
        let mut buf = SseLineBuffer::default();
        let lines = buf.push(b"data: a\r\ndata: b\r\n").unwrap();
        assert_eq!(lines[0], b"data: a\r".to_vec());
        assert_eq!(lines[1], b"data: b\r".to_vec());
    }

    #[test]
    fn an_unterminated_line_is_refused_at_the_ceiling_not_after_it() {
        // The ceiling has to bite while the bytes are still arriving: a buffer
        // that only reports the breach after the stream ends has already
        // accepted whatever the endpoint wanted to send.
        let mut buf = SseLineBuffer::default();
        let filler = vec![b'x'; CAPTURE_CAP_BYTES];
        assert!(buf.push(&filler).is_ok());
        let err = buf.push(b"y").unwrap_err();
        assert!(
            err.to_string().contains("capture ceiling"),
            "the refusal must name the ceiling it hit: {err}"
        );
    }

    #[test]
    fn an_unterminated_line_under_the_ceiling_still_arrives_whole() {
        // The counterpart of the refusal above: the cap must not turn a
        // legitimate long frame into a lost one.
        let mut buf = SseLineBuffer::default();
        let body = format!("data: {}\n", "z".repeat(1024 * 1024));
        let lines = buf.push(body.as_bytes()).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), body.trim_end().len());
        assert!(buf.finish().is_empty());
    }
}
