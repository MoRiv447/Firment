import { describe, expect, it } from 'vitest';
import { WORKFLOW, workflowSteps } from '../steps';
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
