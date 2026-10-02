import { CheckCircle2, XCircle } from 'lucide-react';

import { workflowSteps } from '../lib/steps';
import { turnTimeline } from '../lib/timeline';
import { formatStepDuration } from '../lib/timing';
import { Icon } from '../ui';
import styles from './TurnVerdict.module.css';
import type { ToolCardState } from '../types';

/**
 * What the turn achieved, in one line, at the end of it.
 *
 * The transcript shows every call, but showing them is not the same as finishing:
 * after eight cards a reader wants the sentence, not the receipts. A mark, a
 * clause, and the clock pushed to the far edge -- the same shape the design's own
 * terminal band uses.
 *
 * It says nothing a card above it did not say. Every clause comes from a step that
 * actually ran, and a step that never happened contributes nothing rather than
 * "not run": a chat that only read files has no build to report, and a band
 * listing three things that did not happen reads as a failure log.
 *
 * The wall clock only. Where `TurnTimeline` breaks the same span into tools /
 * waiting / model, that is the answer to "where did the time go", and this band is
 * on the other side of the fold from it -- here the only question is how long.
 */
export function TurnVerdict({
  tools,
  now,
  turnStartedAt,
}: {
  tools: ToolCardState[];
  now: number;
  turnStartedAt?: number;
}) {
  const steps = workflowSteps(tools, now);
  if (!steps) return null;
  const ran = steps.filter((s) => s.state !== 'pending');
  if (ran.length === 0) return null;

  const failed = ran.some((s) => s.state === 'failed');
  const spent = turnTimeline(tools, now, turnStartedAt);
  const clause = ran
    .map((s) => {
      const word = s.state === 'failed' ? 'failed' : s.state === 'done' ? 'passed' : 'running';
      return `${s.label} ${word}`;
    })
    .join(' · ');

  return (
    <div data-ui="turn-verdict" data-tone={failed ? 'failed' : 'ok'} className={styles.band}>
      <Icon src={failed ? XCircle : CheckCircle2} size="sm" tone={failed ? 'failed' : 'brand'} />
      <span className={styles.text}>{clause}</span>
      {spent && (
        <span className={styles.time}>
          {spent.running ? 'running' : 'turn'} {formatStepDuration(spent.wallMs)}
        </span>
      )}
    </div>
  );
}
