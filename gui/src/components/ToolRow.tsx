import { parseDiff } from '../lib/diff';
import { markState } from '../lib/steps';
import { formatStepDuration, timingFor } from '../lib/timing';
import { describeArgs } from '../lib/toolArgs';
import type { ToolCardState } from '../types';
import { StatusMark } from '../ui';
import styles from './ToolRow.module.css';

/**
 * One tool call, as one line.
 *
 * The transcript's other two shapes are the full card (`ToolCard`, with the output
 * in it) and the run's own fold (`MessageList`, `▸ 12 steps · read_file ×4`). This
 * is the third one, and it exists because of what a *live* run needs: not "what did
 * it do" and not "everything it did", but a column you can watch. A card per call
 * during a run is a log scrolling past; one fold for the whole run hides the step
 * that is failing. A 26px row is neither.
 *
 * **The mark is the state and the name is the tool.** The name column is a fixed
 * `min-width`, so a scan down the stack compares tools at one x position instead of
 * chasing an ellipsis; the args then take the rest and truncate, because they are
 * the part that is interesting only once you have found the row you want.
 *
 * A row is a `<button>` when it can open its own card and a `<div>` when it cannot
 * -- the same pairing `ToolCard` uses for its head, and for the same reason: the
 * element has to be one or the other, and making it a button that does nothing is
 * a control that lies.
 */
export function ToolRow({
  tool,
  now,
  open,
  onToggle,
}: {
  tool: ToolCardState;
  /** The caller's clock, so a running row's seconds move without an own ticker. */
  now?: number;
  /** Set together with `onToggle` to make the row fold its card open. */
  open?: boolean;
  onToggle?: () => void;
}) {
  const state = markState(tool.status);
  const timing = timingFor(tool, now ?? Date.now());
  const diff = parseDiff(tool.detail);
  const args = describeArgs(tool.args);
  const running = tool.status === 'running';

  const inner = (
    <>
      <StatusMark state={state} />
      <span className={styles.name}>{tool.name}</span>
      {args ? <span className={styles.args}>{args}</span> : <span className={styles.args} />}
      {/*
       * The phase, when the tool reports one. `§16.2` gates it to two seconds in
       * `ToolCard` and the same gate applies here through the caller's own
       * `showProgress` decision -- a phase that appeared and vanished while you read
       * the row above it is worse than silence.
       */}
      {running && tool.progress ? <span className={styles.phase}>{tool.progress}</span> : null}
      {diff && (diff.added > 0 || diff.removed > 0) ? (
        <span className={styles.counts}>
          <span data-kind="added">+{diff.added}</span>
          <span data-kind="removed">-{diff.removed}</span>
        </span>
      ) : null}
      {/* An em dash, not `0.0s`: a step nobody timed has no duration, and printing
          a zero is a measurement that was never taken. */}
      <span className={styles.time}>{timing ? formatStepDuration(timing.elapsedMs) : '—'}</span>
    </>
  );

  if (!onToggle) {
    return (
      <div data-ui="tool-row" data-state={state} className={styles.row}>
        {inner}
      </div>
    );
  }

  return (
    <button
      type="button"
      data-ui="tool-row"
      data-state={state}
      data-fold="true"
      className={styles.row}
      aria-expanded={open}
      onClick={onToggle}
    >
      {inner}
    </button>
  );
}
