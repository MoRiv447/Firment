import { beforeEach, describe, expect, it } from 'vitest';
import { WORKFLOW, workflowSteps } from '../steps';
import { recordCompleted, resetRuns } from '../timing';
import type { ToolCardState } from '../../types';

/**
 * The step row is derived from the turn's tools, so these tests are the contract
 * between the event stream and what the user sees.
 *
 * The rule they exist to protect: the row never claims progress that did not
 * happen. A step with no tool behind it is `pending`, never `done`, and a
 * failure comes only from a tool that actually reported one.
 */

function tool(
  seq: number,
  name: string,
  status: ToolCardState['status'],
): ToolCardState {
  return { seq, name, args: {}, status };
}

/** The states, in workflow order, for terser assertions. */
function statesOf(tools: ToolCardState[]): string[] {
  return (workflowSteps(tools) ?? []).map((s) => s.state);
}

describe('workflowSteps', () => {
  it('reports nothing for a turn that never builds', () => {
    // An empty build/flash/monitor row on an ordinary chat is furniture.
    expect(workflowSteps([])).toBeNull();
    expect(workflowSteps([tool(1, 'edit_file', 'ok'), tool(2, 'shell', 'ok')])).toBeNull();
  });

  it('shows the whole workflow as soon as one step is involved', () => {
    const steps = workflowSteps([tool(1, 'build', 'running')]);
    expect(steps).not.toBeNull();
    // The shape stays put: a row that grows as the turn proceeds moves under
    // the reader's eyes.
    expect(steps).toHaveLength(WORKFLOW.length);
    expect(steps?.map((s) => s.key)).toEqual(['build', 'flash', 'monitor']);
    expect(steps?.map((s) => s.label)).toEqual(['Build', 'Flash', 'Monitor']);
  });

  it('reads a running tool as the current step', () => {
    expect(statesOf([tool(1, 'build', 'running')])).toEqual([
      'current',
      'pending',
      'pending',
    ]);
  });

  it('walks the workflow as the tools report back', () => {
    expect(
      statesOf([tool(1, 'build', 'ok'), tool(2, 'flash', 'running')]),
    ).toEqual(['done', 'current', 'pending']);

    expect(
      statesOf([tool(1, 'build', 'ok'), tool(2, 'flash', 'ok'), tool(3, 'monitor', 'running')]),
    ).toEqual(['done', 'done', 'current']);

    expect(
      statesOf([tool(1, 'build', 'ok'), tool(2, 'flash', 'ok'), tool(3, 'monitor', 'ok')]),
    ).toEqual(['done', 'done', 'done']);
  });

  it('is red only where a tool actually failed', () => {
    const states = statesOf([tool(1, 'build', 'ok'), tool(2, 'flash', 'failed')]);
    // The failed step is the only one allowed to be red; monitor never ran, so
    // it stays "not yet" rather than inheriting the failure.
    expect(states).toEqual(['done', 'failed', 'pending']);
  });

  it('lets a later attempt supersede an earlier one', () => {
    // A retry is the attempt that counts: build failed, then build passed.
    expect(
      statesOf([tool(1, 'build', 'failed'), tool(2, 'shell', 'ok'), tool(3, 'build', 'ok')]),
    ).toEqual(['done', 'pending', 'pending']);
  });

  it('is not confused by the order the tools arrive in', () => {
    // The arguments are sorted by seq rather than trusted, because a retry is
    // only "later" if the sequence number says so.
    const shuffled = [tool(3, 'monitor', 'running'), tool(1, 'build', 'ok')];
    expect(statesOf(shuffled)).toEqual(['done', 'pending', 'current']);
  });

  it('ignores unrelated tools running in the same turn', () => {
    const states = statesOf([
      tool(1, 'read_file', 'ok'),
      tool(2, 'build', 'ok'),
      tool(3, 'edit_file', 'failed'),
    ]);
    // A failed edit is not a failed build, and the row says nothing about it.
    expect(states).toEqual(['done', 'pending', 'pending']);
  });
});

/**
 * The other half of the row: what it is allowed to say about time.
 *
 * `steps.ts` passes the tools' clocks through and adds nothing of its own, so the
 * cases worth pinning are the ones where a card has no clock at all -- a step that
 * has not started, and one reopened from a transcript.
 */
describe('workflowSteps timings', () => {
  beforeEach(() => {
    resetRuns();
  });

  const timed = (
    seq: number,
    name: string,
    status: ToolCardState['status'],
    startedAt?: number,
    endedAt?: number,
  ): ToolCardState => ({ seq, name, args: {}, status, startedAt, endedAt });

  it('carries the measured duration of a finished step', () => {
    const steps = workflowSteps([timed(1, 'build', 'ok', 1_000, 5_234)], 99_999);
    expect(steps?.[0]).toMatchObject({ key: 'build', state: 'done', elapsedMs: 4_234 });
    // A finished step is not predicted: what is left to estimate is nothing.
    expect(steps?.[0].estimateMs).toBeNull();
  });

  it('keeps a gate wait out of the duration and still puts it on the row', () => {
    // Asserted at the mapping and not only inside `timing`, because this is the
    // seam where the wait could quietly disappear: a row printing eight seconds
    // after the user watched three minutes would leave the missing two minutes
    // unaccounted for, and that is the same lie in another shape.
    const gated = { ...timed(1, 'build', 'ok', 1_000, 129_000), waitedMs: 120_000 };
    expect(workflowSteps([gated], 99_999)?.[0]).toMatchObject({
      elapsedMs: 8_000,
      waitedMs: 120_000,
    });
  });

  it('reports no wait for a step nobody was asked about', () => {
    // `null` rather than `0`: the row's note is about a dialog that happened, and
    // an auto-approved build never opened one.
    const steps = workflowSteps([timed(1, 'build', 'ok', 1_000, 5_000)], 99_999);
    expect(steps?.[0]).toMatchObject({ elapsedMs: 4_000, waitedMs: null });
  });

  it('counts a running step up to the caller\u2019s clock', () => {
    const steps = workflowSteps([timed(1, 'flash', 'running', 10_000)], 13_500);
    expect(steps?.[1]).toMatchObject({ key: 'flash', state: 'current', elapsedMs: 3_500 });
    // No history for `flash` in this session, so no estimate is offered.
    expect(steps?.[1].estimateMs).toBeNull();
  });

  it('offers an estimate once the same tool has been watched finish twice', () => {
    recordCompleted([
      timed(1, 'build', 'ok', 0, 4_000),
      timed(2, 'build', 'ok', 0, 6_000),
    ], 's');
    const steps = workflowSteps([timed(3, 'build', 'running', 1_000)], 3_000);
    expect(steps?.[0]).toMatchObject({ state: 'current', elapsedMs: 2_000, estimateMs: 5_000 });
  });

  it('says nothing about a step nobody timed', () => {
    // A reopened transcript card: the tool was called, but the card has neither a
    // start nor an end, so there is no duration to show and none to invent.
    const steps = workflowSteps([timed(1, 'build', 'unknown')], 99_999);
    expect(steps?.[0]).toMatchObject({ key: 'build', state: 'pending' });
    expect(steps?.[0].elapsedMs).toBeUndefined();
  });
});
