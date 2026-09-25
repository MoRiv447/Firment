import { formatStepDuration } from '../lib/timing';
import styles from './StepProgress.module.css';

/**
 * The build / flash / monitor progress row.
 *
 * Deliberately **not** buttons. A step is a report of where the work got to, so
 * the whole row is non-interactive: no hover, no pointer cursor, no click
 * target. Letting it look pressable is how a progress row turns into a control
 * that does nothing.
 *
 * The four states are visually distinct, and the two rules that matter come
 * from docs/design/tokens.md:
 *
 * * **Red is reserved for real errors.** `unknown` is not a failure -- it is a
 *   neutral `o`, never a `x`. A step whose outcome nobody has measured yet must
 *   not read as a step that failed. The one state that may be red is `failed`,
 *   and it is only ever reached from a tool that actually reported failure.
 * * **A pending step is legible, not greyed out.** Its label stays readable
 *   (4.51:1 on the light ground), so it reads as "not yet", not as "unavailable".
 *
 * Colour per state, both schemes:
 *
 *   done     filled chip -- success pair (6.49:1 dark / 6.19:1 light)
 *   failed   filled chip -- the removed-diff pair (6.56:1 light)
 *   current  no fill, body ink, 2px brand rule underneath
 *   pending  no fill, muted ink
 *   unknown  no fill, muted ink, `o` glyph
 *
 * `data-state` carries all of it, which is the whole point of the rewrite: the
 * previous version computed `background`, `color` and `borderBottom` from
 * `styles/tokens.ts` in JS, so a step row kept the colours of whichever scheme the
 * token cache had resolved when it first rendered, and switching the OS to light
 * mid-session left the progress row behind in the dark.
 *
 * The numbers follow one rule (`lib/timing.ts`): **measured, or absent.** A finished
 * step reports what it took, a running one counts up, and `~4.0s` sits beside the
 * running step only when this session has already watched the same tool finish at
 * least twice. A step that has not started gets no number -- the row reports, it does
 * not forecast.
 */

export type StepState = 'done' | 'current' | 'pending' | 'unknown' | 'failed';

export interface StepProgressItem {
  /** Stable key; also the accessible label when `label` is absent. */
  key: string;
  label?: string;
  state: StepState;
  /** Measured: the tool's own time — the whole run for a finished step, so far for
   * a running one, and never the reader's time at a permission dialog. */
  elapsedMs?: number;
  /**
   * What the permission gate cost the person, or `null` when nobody was asked.
   * Printed beside `elapsedMs` rather than inside it, and under a second not at all:
   * a reader who watched three minutes go by needs the other two accounted for, and
   * a step that was approved instantly has nothing to explain.
   */
  waitedMs?: number | null;
  /**
   * What this step is likely to take, from this session's completed runs of the
   * same tool. `null` or absent when there is no such history — the row shows the
   * elapsed time alone rather than a made-up number.
   */
  estimateMs?: number | null;
}

/** The glyph for a state. `unknown` is `o`, never `x` -- unknown is not failed. */
function glyph(state: StepState): string {
  switch (state) {
    case 'done':
      return '✓';
    case 'current':
      return '◐';
    case 'unknown':
      return '○';
    case 'failed':
      return '✕';
    default:
      return '·';
  }
}

/** Screen-reader text: the glyph alone is not a state anyone can hear. */
const STATE_WORDS: Record<StepState, string> = {
  done: 'done',
  current: 'in progress',
  pending: 'not started',
  unknown: 'not yet measured',
  failed: 'failed',
};

/** Below this, a wait explains nothing: an answer that came back at once is not a
 * gap the reader noticed. */
const WAIT_NOTE_FLOOR_MS = 1_000;

/** The gate time worth naming, or `null` when there is nothing to say. */
function notableWait(ms: number | null | undefined): number | null {
  return ms != null && ms >= WAIT_NOTE_FLOOR_MS ? ms : null;
}

/**
 * The whole row, spoken.
 *
 * The numbers are as invisible to a screen reader as the glyph is, so they are said
 * out loud here -- and said for what they are: a finished step *took* its duration,
 * a running one has been going that long, and an estimate is prefixed `about` rather
 * than stated. The waiting time is named too, because a step that says `8s` after
 * three minutes on screen otherwise leaves the reader to wonder which number lied.
 */
function spoken(item: StepProgressItem): string {
  const name = `${item.label ?? item.key}: ${STATE_WORDS[item.state]}`;
  if (item.elapsedMs === undefined) return name;

  const waited = notableWait(item.waitedMs);
  const waitClause = waited === null ? '' : `, after ${formatStepDuration(waited)} waiting`;
  const elapsed = formatStepDuration(item.elapsedMs);
  if (item.state !== 'current') return `${name}, took ${elapsed}${waitClause}`;
  if (item.estimateMs == null) return `${name} for ${elapsed}${waitClause}`;
  return `${name} for ${elapsed}, about ${formatStepDuration(item.estimateMs)} expected${waitClause}`;
}

export function StepProgress({ steps }: { steps: StepProgressItem[] }) {
  return (
    <div data-ui="step-progress" role="list" className={styles.root}>
      {steps.map((step) => {
        const waited = notableWait(step.waitedMs);
        return (
          <span
            key={step.key}
            data-ui="step-item"
            data-state={step.state}
            role="listitem"
            aria-current={step.state === 'current' ? 'step' : undefined}
            aria-label={spoken(step)}
            className={styles.step}
          >
            <span aria-hidden className={styles.mark}>
              {glyph(step.state)}
            </span>
            <span>{step.label ?? step.key}</span>
            {/*
              * A duration only where one was measured, and an estimate only beside a
              * running step. A step that has not started carries neither: the row
              * reports, it does not forecast.
              */}
            {step.elapsedMs !== undefined && (
              <span className={styles.time} aria-hidden>
                {formatStepDuration(step.elapsedMs)}
                {waited !== null && ` +${formatStepDuration(waited)} waiting`}
                {step.state === 'current' &&
                  step.estimateMs != null &&
                  ` · ~${formatStepDuration(step.estimateMs)}`}
              </span>
            )}
          </span>
        );
      })}
    </div>
  );
}
