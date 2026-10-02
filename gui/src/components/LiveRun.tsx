import { Fragment, useState } from 'react';

import type { ToolCardState } from '../types';
import { ToolCard } from './ToolCard';
import { ToolRow } from './ToolRow';
import { TurnTimeline } from './TurnTimeline';
import styles from './LiveRun.module.css';
import stack from './toolStack.module.css';

/**
 * A live turn's tool work, as a column of one-line rows.
 *
 * What this replaced was a single folded line -- `▸ ⏺ edit_file · 12s · 4 steps ·
 * read_file ×4` -- whose argument was that five cards scrolling past is a log, and
 * `edit_file · 12s` is a status. That was right about the two shapes it compared,
 * and wrong about the third: a *card* per call is a log, a fold for the whole run
 * hides the step that is failing, and a 26px row is neither. The row is the one
 * shape that answers "what is it doing now" at the cost of a line instead of a
 * screenful, and it is what the landing page's own demo does.
 *
 * **One card open at a time, and opening is per row.** The output is still the
 * reason the rows exist -- a `read_file` answer is up to 2000 characters -- so a
 * row opens its own card underneath itself and the rest of the column stays a
 * column. A single `openSeq` rather than a set: two open cards would push the step
 * being watched off the screen, which is the one thing this layout is for.
 *
 * The timeline stays, at the bottom, where it now reads as the run's total rather
 * than as a detail hidden behind a fold.
 */
export function LiveRun({
  tools,
  now,
  onAction,
  onOpenChanges,
  turnStartedAt,
}: {
  tools: ToolCardState[];
  /**
   * The caller's clock, required.
   *
   * This component used to keep one of its own -- an interval that ticked while
   * something was in flight -- and that made it the only place in the app that read
   * the time instead of being handed it. `workflowSteps(tools, now)` and
   * `TurnTimeline now={…}` both take it for the reason written in `lib/steps.ts`:
   * a component that calls `Date.now()` during render only moves when something
   * else causes a render, and one that keeps its own interval cannot be drawn at a
   * fixed instant -- which is how the gallery ended up printing a two-year-old run
   * as `25257h 51m`. One clock, owned by the caller that already needs it.
   */
  now: number;
  onAction?: (prompt: string) => void;
  onOpenChanges?: () => void;
  /**
   * When the turn began, which is earlier than the first row: the stretch between
   * the two is the model answering, and a timeline that started at the first call
   * would quietly drop the longest part of some turns.
   */
  turnStartedAt?: number;
}) {
  const [openSeq, setOpenSeq] = useState<number | null>(null);

  const sorted = [...tools].sort((a, b) => a.seq - b.seq);
  if (sorted.length === 0) return null;

  return (
    <div data-ui="live-run" className={styles.root}>
      <div className={stack.stack}>
        {sorted.map((tool) => (
          <Fragment key={tool.seq}>
            <ToolRow
              tool={tool}
              now={now}
              open={openSeq === tool.seq}
              onToggle={() => setOpenSeq((s) => (s === tool.seq ? null : tool.seq))}
            />
            {openSeq === tool.seq && (
              <div className={stack.card}>
                <ToolCard tool={tool} onAction={onAction} onOpenChanges={onOpenChanges} />
              </div>
            )}
          </Fragment>
        ))}
      </div>
      <TurnTimeline tools={sorted} now={now} turnStartedAt={turnStartedAt} />
    </div>
  );
}
