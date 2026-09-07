import { describe, expect, it } from 'vitest';
import { STALL_NOTICE_SECS, shouldShowStallNotice, stallNotice } from '../stallHint';

describe('stall notice', () => {
  it('never fires before the backend gives up on the stream', () => {
    // agent.rs fails a stream after 120s without bytes (stream_timeout_secs).
    // The old 60s threshold warned the user "the stream may be stalled" while
    // the agent was mid-way through a normal, slow reply.
    const BACKEND_STREAM_TIMEOUT_SECS = 120;
    expect(STALL_NOTICE_SECS).toBeGreaterThan(BACKEND_STREAM_TIMEOUT_SECS);
  });

  it('waits for the threshold instead of firing at it one second early', () => {
    expect(shouldShowStallNotice(STALL_NOTICE_SECS - 1)).toBe(false);
    expect(shouldShowStallNotice(STALL_NOTICE_SECS)).toBe(true);
  });

  it('reports the real idle time and points at what happens next', () => {
    const { message, description } = stallNotice(180);
    expect(message).toContain('180s');
    expect(message).not.toContain('60s');
    expect(description).toContain('Stop');
  });
});
