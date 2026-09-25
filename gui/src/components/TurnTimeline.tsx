import type { CSSProperties } from 'react';

import { describeTurnTime, turnTimeline } from '../lib/timeline';
import { formatStepDuration } from '../lib/timing';
import type { ToolCardState } from '../types';
import styles from './TurnTimeline.module.css';

/**
 * How a turn's wall clock was spent, above the cards it describes.
 *
 * The run line already answers "what is happening now". This answers the question
 * people ask afterwards: the turn took three minutes, and how much of that was the
 * compiler, how much was the model, and how much was a dialog waiting for an answer.
 * Each card carries its own duration, but three dozen of them do not add up to a
 * total anyone can read — and the addition is not even defined, because calls run
 * concurrently and their windows overlap.
 *
 * So the arithmetic lives in `lib/timeline` (union, priority, and an invariant that
 * the parts sum to the whole), and this component only paints proportions and says
 * the sentence out loud.
 *
 * It renders nothing when no card has a clock: a run reopened from a stored
 * transcript knows what ran and never how long it took, and a bar drawn from
 * unknowns would be the same claim the cards already refuse to make.
 */
export function TurnTimeline({
  tools,
  now,
  turnStartedAt,
}: {
  tools: readonly ToolCardState[];
  /** The caller's clock, so a tick re-renders one component rather than the tree. */
  now: number;
  /** The turn's own start — without it the stretch before the first call is outside the bar. */
  turnStartedAt?: number;
}) {
  const spent = turnTimeline(tools, now, turnStartedAt);
  if (!spent) return null;
  // A share of the wall clock, handed to the sheet as a custom property: the rule
  // here is that a computed number travels in a variable and CSS decides what to do
  // with it (`TodosPane`'s progress, `Slider`'s fill), because an inline `width` is
  // a declaration no stylesheet can override for a narrow column or a theme.
  const share = (ms: number) => ({ '--share': `${(ms / spent.wallMs) * 100}%` } as CSSProperties);
  const sentence = describeTurnTime(spent, formatStepDuration);

  return (
    <div data-ui="turn-timeline" className={styles.root}>
      <span aria-hidden className={styles.bar}>
        <span className={styles.work} style={share(spent.workMs)} />
        <span className={styles.wait} style={share(spent.waitedMs)} />
      </span>
      <span className={styles.legend}>{sentence}</span>
    </div>
  );
}
