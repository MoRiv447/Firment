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
 */

export type StepState = 'done' | 'current' | 'pending' | 'unknown' | 'failed';

export interface StepProgressItem {
  /** Stable key; also the accessible label when `label` is absent. */
  key: string;
  label?: string;
  state: StepState;
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

export function StepProgress({ steps }: { steps: StepProgressItem[] }) {
  return (
    <div data-ui="step-progress" role="list" className={styles.root}>
      {steps.map((step) => (
        <span
          key={step.key}
          data-ui="step-item"
          data-state={step.state}
          role="listitem"
          aria-current={step.state === 'current' ? 'step' : undefined}
          aria-label={`${step.label ?? step.key}: ${STATE_WORDS[step.state]}`}
          className={styles.step}
        >
          <span aria-hidden className={styles.mark}>
            {glyph(step.state)}
          </span>
          <span>{step.label ?? step.key}</span>
        </span>
      ))}
    </div>
  );
}
