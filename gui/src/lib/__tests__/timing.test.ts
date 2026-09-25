import { beforeEach, describe, expect, it } from 'vitest';
import {
  estimateFor,
  formatStepDuration,
  recordCompleted,
  resetRuns,
  timingFor,
} from '../timing';
import type { ToolCardState } from '../../types';

/**
 * The contract between the clock and the row: a duration is measured or it is
 * absent, and an estimate exists only when this session has actually watched the
 * tool finish more than once.
 *
 * These are the tests that keep a future change from turning `~4.0s` into a guess:
 * every path that has no history behind it is asserted to produce `null`, because
 * the failure mode here is not a wrong number, it is a confident one.
 */

function ran(seq: number, name: string, ms: number): ToolCardState {
  return {
    seq,
    name,
    args: {},
    status: 'ok',
    startedAt: 1_000,
    endedAt: 1_000 + ms,
  };
}

beforeEach(() => {
  resetRuns();
});

describe('the run ledger', () => {
  it('counts a finished run', () => {
    recordCompleted([ran(1, 'build', 4_000)], 's');
    // One sample is not a median, so there is still no estimate to give.
    expect(estimateFor('build')).toBeNull();
  });

  it('counts each run once however many times it is offered', () => {
    // The caller records on every render, so this is the property that keeps a
    // re-render from counting as evidence.
    const tools = [ran(1, 'build', 4_000), ran(2, 'build', 6_000)];
    recordCompleted(tools, 's');
    recordCompleted(tools, 's');
    recordCompleted(tools, 's');
    // Two distinct runs, not six: the median of 4000/6000 is 5000 either way, but a
    // ledger that inflated would eventually push real samples out of the window.
    expect(estimateFor('build')).toBe(5_000);
  });

  it('ignores a run that has not ended, and one that never started', () => {
    recordCompleted([
      { seq: 1, name: 'build', args: {}, status: 'running', startedAt: 1_000 },
      // A card reopened from a transcript: the tool was called, nobody timed it.
      { seq: 2, name: 'flash', args: {}, status: 'unknown' },
    ], 's');
    expect(estimateFor('build')).toBeNull();
    expect(estimateFor('flash')).toBeNull();
  });

  it('takes the median, so one cold build does not speak for the tool', () => {
    for (const ms of [4_000, 90_000, 4_200]) recordCompleted([ran(ms, 'build', ms)], 's');
    // A mean would be 32.7s here, which is a number no build in this session took.
    expect(estimateFor('build')).toBe(4_200);
  });

  it('averages the middle pair when the count is even', () => {
    for (const ms of [4_000, 5_000]) recordCompleted([ran(ms, 'build', ms)], 's');
    expect(estimateFor('build')).toBe(4_500);
  });

  it('keeps the recent runs and forgets the old ones', () => {
    for (let i = 0; i < 10; i++) recordCompleted([ran(i, 'build', (i + 1) * 1_000)], 's');
    // Ten offered, eight kept, and it is the *oldest* two that fall out: the median
    // of the window (3s..10s) is 6.5s, where all ten would have given 5.5s. Pinning
    // the number rather than "roughly" is what makes the direction of the window a
    // decision someone has to change on purpose.
    expect(estimateFor('build')).toBe(6_500);
  });

  it('counts a new session\u2019s runs even though its seqs start over', () => {
    // `seq` is the agent's own counter and restarts at 1 with every agent. Deduping on
    // the number alone made a whole session's run look like repeats of the first
    // session's -- the ledger stopped learning and the estimate never appeared again,
    // which is the failure the scope argument exists to prevent.
    recordCompleted([ran(1, 'build', 4_000), ran(2, 'build', 6_000)], 'session-a');
    recordCompleted([ran(1, 'build', 3_000), ran(2, 'build', 5_000)], 'session-b');
    // Four runs, not two: 3000/4000/5000/6000.
    expect(estimateFor('build')).toBe(4_500);
  });

  it('counts a second turn of one session, whose calls are numbered after the first', () => {
    // The same dedup, one level down. The GUI builds a fresh `Agent` for every turn, so
    // while the call counter lived on the agent each turn restarted at 1 and every run of
    // turn two looked like a repeat already counted: the ledger stopped learning after the
    // first turn of a chat. The counter is the session's now, so the stream continues.
    recordCompleted([ran(1, 'build', 4_000), ran(2, 'build', 6_000)], 'same-session');
    recordCompleted([ran(3, 'build', 3_000), ran(4, 'build', 5_000)], 'same-session');
    // Four runs, not two: 3000/4000/5000/6000.
    expect(estimateFor('build')).toBe(4_500);
  });

  it('keeps the tools apart', () => {
    for (const ms of [1_000, 3_000]) recordCompleted([ran(ms, 'build', ms)], 's');
    for (const ms of [20_000, 30_000]) recordCompleted([ran(ms, 'flash', ms)], 's');
    expect(estimateFor('build')).toBe(2_000);
    expect(estimateFor('flash')).toBe(25_000);
  });
});

describe('timingFor', () => {
  it('has nothing to say about a step that was never timed', () => {
    expect(timingFor({ seq: 1, name: 'flash', args: {}, status: 'unknown' }, 5_000)).toBeNull();
  });

  it('counts up while the step runs, and stops when it ends', () => {
    const running: ToolCardState = {
      seq: 1,
      name: 'build',
      args: {},
      status: 'running',
      startedAt: 1_000,
    };
    expect(timingFor(running, 4_500)?.elapsedMs).toBe(3_500);
    // The same card, one second later: the clock is the caller's, not the card's.
    expect(timingFor(running, 5_500)?.elapsedMs).toBe(4_500);

    const finished = { ...running, status: 'ok' as const, endedAt: 4_000 };
    expect(timingFor(finished, 99_999)).toMatchObject({
      elapsedMs: 3_000,
      finished: true,
      estimateMs: null,
    });
  });

  it('offers an estimate only to a running step with history behind it', () => {
    for (const ms of [4_000, 6_000]) recordCompleted([ran(ms, 'build', ms)], 's');
    const running: ToolCardState = {
      seq: 99,
      name: 'build',
      args: {},
      status: 'running',
      startedAt: 10_000,
    };
    expect(timingFor(running, 11_000)).toMatchObject({ elapsedMs: 1_000, estimateMs: 5_000 });
    // A step that never had a twin in this session gets no number: the flash above
    // has no history, so its running card reports elapsed time and nothing else.
    const flash: ToolCardState = { ...running, name: 'flash' };
    expect(timingFor(flash, 11_000)?.estimateMs).toBeNull();
  });

  it('never reports a negative elapsed, whatever the clocks say', () => {
    // NTP stepping the wall clock backwards is the real case: the row must not
    // print `-2.0s`.
    const card: ToolCardState = {
      seq: 1,
      name: 'build',
      args: {},
      status: 'running',
      startedAt: 5_000,
    };
    expect(timingFor(card, 3_000)?.elapsedMs).toBe(0);
  });
});

describe('the permission gate', () => {
  // A card is timed from `tool_start` to `tool_end`, and the gate sits between those
  // two. Everything below is the same fact seen from the two places that must agree:
  // the number under the step, and the sample the next estimate is built from.
  function gated(seq: number, name: string, wallMs: number, waitedMs: number): ToolCardState {
    return { ...ran(seq, name, wallMs), waitedMs };
  }

  it('takes the human time off the step and names it beside the tool', () => {
    const card = gated(1, 'flash', 128_000, 120_000);
    expect(timingFor(card, 999_999)).toMatchObject({
      elapsedMs: 8_000,
      waitedMs: 120_000,
    });
  });

  it('leaves a step nobody asked about at its full length', () => {
    // `null` is the common case (an auto-approve rule, a read-only tool), and it must
    // not behave like a zero that was measured — nor shorten anything.
    expect(timingFor(ran(1, 'build', 4_000), 999_999)).toMatchObject({
      elapsedMs: 4_000,
      waitedMs: null,
    });
    expect(timingFor(gated(1, 'build', 4_000, 0), 999_999)?.elapsedMs).toBe(4_000);
  });

  it('learns the tool time rather than the time spent reading a dialog', () => {
    // The failure this exists to prevent: one long look at a dialog, and every later
    // `flash` in the session is estimated at two minutes.
    recordCompleted([gated(1, 'flash', 128_000, 120_000)], 's');
    recordCompleted([gated(2, 'flash', 130_000, 122_000)], 's');
    expect(estimateFor('flash')).toBe(8_000);
  });

  it('cannot be talked into a negative duration by an impossible report', () => {
    // A wait longer than the card's own life should not exist. If one is ever
    // reported, the answer is "the tool took no measurable time", not `-2.0s`.
    expect(timingFor(gated(1, 'build', 1_000, 60_000), 999_999)?.elapsedMs).toBe(0);
  });
});

describe('formatStepDuration', () => {
  it('keeps a tenth of a second where a whole one would read as nothing', () => {
    expect(formatStepDuration(0)).toBe('<0.1s');
    expect(formatStepDuration(99)).toBe('<0.1s');
    expect(formatStepDuration(400)).toBe('0.4s');
    expect(formatStepDuration(4_234)).toBe('4.2s');
  });

  it('switches to whole seconds exactly once, without a 10.0s', () => {
    expect(formatStepDuration(9_900)).toBe('9.9s');
    expect(formatStepDuration(9_999)).toBe('10s');
    expect(formatStepDuration(10_000)).toBe('10s');
    // Above that it is the same shape the run timer uses, so the two labels on
    // screen cannot spell the same duration two ways.
    expect(formatStepDuration(65_000)).toBe('1m 05s');
    expect(formatStepDuration(3_600_000)).toBe('1h 00m');
  });
});
