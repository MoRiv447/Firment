import type { MonitorLine } from '../types';

/** How many lines of serial output one port keeps on screen. */
export const MONITOR_LINE_CAP = 2000;

/**
 * Fold a batch of monitor lines into the per-port state.
 *
 * The batch is the unit because the serial monitor emits ONE event per line: applied straight to
 * state, a board that boots noisily rebuilt the whole shell — transcript and every pane beside it —
 * hundreds of times a second. So `App` holds the lines for 50 ms and folds them here, the same
 * token-burst shape the agent-event stream already uses.
 *
 * The cap is applied once per port AFTER the appends rather than once per line inside them. Both
 * keep the same last `MONITOR_LINE_CAP` lines; the difference is that the loop does not have to
 * know where the batch ends, and a port touched twice in one flush is rebuilt once.
 *
 * The map comes back untouched when the batch is empty. A fold that always built a new object would
 * re-render the monitor for a flush that carried nothing, which is the thing this replaced.
 */
export function appendMonitorLines(
  prev: Record<string, MonitorLine[]>,
  batch: readonly MonitorLine[],
): Record<string, MonitorLine[]> {
  if (batch.length === 0) return prev;
  const next = { ...prev };
  for (const line of batch) next[line.port] = [...(next[line.port] ?? []), line];
  for (const port of Object.keys(next)) {
    if (next[port].length > MONITOR_LINE_CAP) next[port] = next[port].slice(-MONITOR_LINE_CAP);
  }
  return next;
}
