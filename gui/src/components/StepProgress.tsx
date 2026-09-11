import { color, font, radius } from '../styles/tokens';

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

function ink(state: StepState): string {
  switch (state) {
    case 'done':
      return color.stepDoneInk;
    case 'failed':
      return color.stepFailedInk;
    case 'current':
      return color.stepCurrentInk;
    default:
      return color.stepPendingInk;
  }
}

/**
 * Only a finished step and a failed one are filled.
 *
 * The fill is what makes those two scannable without reading the labels, and
 * they are the only two states whose outcome is already known.
 */
function fill(state: StepState): string {
  switch (state) {
    case 'done':
      return color.stepDoneBg;
    case 'failed':
      return color.stepFailedBg;
    default:
      return 'transparent';
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
    <div
      role="list"
      style={{
        display: 'flex',
        alignItems: 'stretch',
        flexWrap: 'wrap',
        gap: 12,
        fontFamily: font.sans,
      }}
    >
      {steps.map((step) => {
        const filled = step.state === 'done' || step.state === 'failed';
        return (
          <div
            key={step.key}
            role="listitem"
            aria-current={step.state === 'current' ? 'step' : undefined}
            aria-label={`${step.label ?? step.key}: ${STATE_WORDS[step.state]}`}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              padding: '4px 10px',
              background: fill(step.state),
              color: ink(step.state),
              // A filled chip needs its chip radius; a bare one has no corners
              // of its own to round.
              borderRadius: filled ? radius.chip : 0,
              fontSize: 13,
              fontWeight: step.state === 'current' ? 600 : 500,
              // The current step's only chrome: a 2px brand rule underneath.
              borderBottom:
                step.state === 'current' ? `2px solid ${color.stepRule}` : '2px solid transparent',
              // Non-interactive on purpose -- see the component note.
              cursor: 'default',
            }}
          >
            <span aria-hidden style={{ opacity: step.state === 'pending' ? 0.7 : 1 }}>
              {glyph(step.state)}
            </span>
            <span>{step.label ?? step.key}</span>
          </div>
        );
      })}
    </div>
  );
}
