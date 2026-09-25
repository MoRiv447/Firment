import { describe, expect, it } from 'vitest';
import { describeTurnTime, turnTimeline } from '../timeline';
import { formatStepDuration } from '../timing';
import type { ToolCardState } from '../../types';

/**
 * The turn's wall clock, attributed once.
 *
 * Every case here exists because a naive version gets it wrong in a specific way:
 * summing durations breaks the moment two calls overlap, taking the last card's end
 * as the start hides the longest model answer of the turn, and letting a finished
 * turn's total grow with the wall clock turns a history line into a lie. The
 * invariant that catches most of them is the same one the reader sees: the three
 * parts add up to the whole.
 */

function card(
  seq: number,
  name: string,
  startedAt?: number,
  endedAt?: number,
  waitedMs?: number | null,
  status: ToolCardState['status'] = 'ok',
): ToolCardState {
  return { seq, name, args: {}, status, startedAt, endedAt, waitedMs };
}

describe('turnTimeline', () => {
  it('adds up, when nothing is timed, there is nothing to say', () => {
    // A run reopened from a stored transcript: the cards exist, the clocks do not.
    expect(turnTimeline([card(1, 'build'), card(2, 'flash')], 10_000)).toBeNull();
  });

  it('counts serial calls and the gaps between them', () => {
    const t = turnTimeline(
      [card(1, 'build', 1_000, 5_000), card(2, 'flash', 6_000, 9_000)],
      99_000,
      0,
    );
    expect(t).toEqual({
      wallMs: 9_000,
      waitedMs: 0,
      workMs: 7_000,
      otherMs: 2_000,
      running: false,
    });
  });

  it('does not double-count two calls that ran at the same time', () => {
    // The failure a sum of durations has: 10 000 + 10 000 ms of "work" inside a
    // 15 000 ms turn. Concurrent `task` calls make this the normal shape, not the
    // corner.
    const t = turnTimeline(
      [card(1, 'build', 0, 10_000), card(2, 'read_file', 5_000, 15_000)],
      99_000,
    );
    expect(t?.wallMs).toBe(15_000);
    expect(t?.workMs).toBe(15_000);
    expect(t?.otherMs).toBe(0);
  });

  it('takes the wait off the window when a call spent its time on a dialog', () => {
    const t = turnTimeline([card(1, 'flash', 1_000, 129_000, 120_000)], 999_999);
    expect(t).toEqual({
      wallMs: 128_000,
      waitedMs: 120_000,
      workMs: 8_000,
      otherMs: 0,
      running: false,
    });
  });

  it('says waiting when one call is blocked and another is working', () => {
    // The judgement this module has to make and state out loud: while a dialog is
    // open, the reader is the reason nothing is finished, even if a build is running
    // underneath it. Work that overlaps a wait is not lost from the cards — each
    // still reports its own duration — it only loses the tie-break here.
    const t = turnTimeline(
      [card(1, 'flash', 0, 10_000, 10_000), card(2, 'build', 2_000, 8_000)],
      99_000,
    );
    expect(t).toEqual({
      wallMs: 10_000,
      waitedMs: 10_000,
      workMs: 0,
      otherMs: 0,
      running: false,
    });
  });

  it('counts a running call up to now, and a finished turn not at all', () => {
    const running = turnTimeline(
      [card(1, 'build', 1_000, undefined, null, 'running')],
      6_000,
      0,
    );
    expect(running).toMatchObject({ wallMs: 6_000, workMs: 5_000, running: true });

    // The same turn after it ended: asking a minute later must not stretch it.
    const done = turnTimeline([card(1, 'build', 1_000, 6_000)], 60_000, 0);
    expect(done).toMatchObject({ wallMs: 6_000, workMs: 5_000, running: false });
  });

  it('keeps the model time before the first call inside the window', () => {
    // Without the turn's own start the 70 s of answering before anything ran would
    // sit outside the bar, and the one number a reader checks would read as a fast
    // turn.
    const withTurn = turnTimeline([card(1, 'build', 70_000, 75_000)], 99_000, 0);
    const without = turnTimeline([card(1, 'build', 70_000, 75_000)], 99_000);
    expect(withTurn).toMatchObject({ wallMs: 75_000, otherMs: 70_000, workMs: 5_000 });
    expect(without).toMatchObject({ wallMs: 5_000, otherMs: 0, workMs: 5_000 });
  });

  it('adds up to the wall clock over an overlap of every kind', () => {
    const mixed = [
      card(1, 'build', 0, 12_000, 4_000),
      card(2, 'flash', 8_000, 20_000, null, 'running'),
      card(3, 'task', 15_000, 18_000, 2_000),
    ];
    const t = turnTimeline(mixed, 20_000, 0);
    expect(t).not.toBeNull();
    expect(t!.waitedMs + t!.workMs + t!.otherMs).toBe(t!.wallMs);
  });
});

describe('describeTurnTime', () => {
  it('names the parts that took time and stays silent about the ones that did not', () => {
    expect(
      describeTurnTime(
        { wallMs: 128_000, waitedMs: 120_000, workMs: 8_000, otherMs: 0, running: false },
        formatStepDuration,
      ),
    ).toBe('total 2m 08s · tools 8.0s · waiting on you 2m 00s');
  });

  it('says running while the turn is still moving', () => {
    expect(
      describeTurnTime(
        { wallMs: 5_000, waitedMs: 0, workMs: 5_000, otherMs: 0, running: true },
        formatStepDuration,
      ),
    ).toBe('running 5.0s · tools 5.0s');
  });
});
