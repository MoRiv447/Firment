import type { ToolCardState } from '../types';
import type { StepProgressItem, StepState } from '../components/StepProgress';

/**
 * The embedded workflow, derived from the tools a turn actually ran.
 *
 * The step row reports where the work got to, so it has to be computed from the
 * tool events rather than kept as a second piece of state that can disagree
 * with them. The data was already there: a turn's `tools` is exactly the set of
 * build / flash / monitor calls, in order.
 *
 * Nothing here invents progress. A step that never ran is `pending` -- the row
 * shows the whole shape so it does not jump around as the turn proceeds, and a
 * step whose tool failed is `failed`, which is the one state allowed to be red.
 */

/** The workflow, in the order it has to happen. The names are the tool names. */
export const WORKFLOW: readonly { name: string; label: string }[] = [
  { name: 'build', label: 'Build' },
  { name: 'flash', label: 'Flash' },
  { name: 'monitor', label: 'Monitor' },
] as const;

function stateFor(status: ToolCardState['status']): StepState {
  switch (status) {
    case 'running':
      return 'current';
    case 'ok':
      return 'done';
    case 'unknown':
      // Reopened history. `default` below means "the turn reported a failure",
      // and an unrecorded outcome is not one.
      return 'pending';
    default:
      return 'failed';
  }
}

/**
 * The row for a turn, or `null` when the turn has nothing to report.
 *
 * `null` rather than three pending steps: a chat that never builds anything
 * should not carry an empty build/flash/monitor row. That is the "unconfigured
 * -> hidden entirely" rule in docs/design/tokens.md, applied to a turn.
 */
export function workflowSteps(tools: ToolCardState[]): StepProgressItem[] | null {
  const byName = new Map<string, ToolCardState>();
  for (const tool of [...tools].sort((a, b) => a.seq - b.seq)) {
    // Last wins: a second build after a failure is the attempt that counts.
    if (WORKFLOW.some((step) => step.name === tool.name)) byName.set(tool.name, tool);
  }
  if (byName.size === 0) return null;

  return WORKFLOW.map((step) => {
    const tool = byName.get(step.name);
    return {
      key: step.name,
      label: step.label,
      state: tool ? stateFor(tool.status) : 'pending',
    };
  });
}
