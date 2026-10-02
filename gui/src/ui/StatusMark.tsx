import styles from './StatusMark.module.css';

/**
 * One of four rounded-square marks, each meaning a state without a word.
 *
 * The vocabulary this replaces is `StatusDot` plus a lucide circle: the dot is a
 * 6px circle and says "alive or not", and `CircleCheck` / `CircleX` are 24px-grid
 * icons that carry their own stroke weight and their own idea of a corner. Neither
 * reads at a glance in a column of rows, which is where this one is used.
 *
 * All four share one geometry -- a 13px rounded square, `rx` half its width, a
 * 1.5px stroke -- so only the thing inside it changes. That is the whole point:
 * a scan down a column of tool rows should find the states by their *shapes*
 * (tick, arrow, cross, empty box) and not by comparing four different line widths.
 *
 * `--brand-ink` is the resting ink, not the acid: a 1.5px stroke needs the value
 * measured as text (`gui/src/ui/Icon.tsx` carries the same note). `pending` is the
 * one exception and takes an edge token, because "not yet" is not a state of the
 * brand -- it is the absence of one.
 *
 * `label` is optional and omitted by most call sites: a tool row already names its
 * tool and its duration next to this, and an `aria-label` there would say the same
 * fact twice. The verdict band passes one, because there a mark stands alone.
 */
export type MarkState = 'done' | 'current' | 'pending' | 'failed';

/** The inner path per state. Exported for the test that reads them. */
export const MARK_PATHS: Record<MarkState, string | null> = {
  // A tick.
  done: 'M4.2 7.2l2 2 3.6-4',
  // A filled triangle: this is happening now, which is also why it is the only
  // filled one -- a fill reads as "live" at a size where a stroke does not.
  current: 'M4.7 4.7l4.6 2.3-4.6 2.3z',
  // A cross.
  failed: 'M4.6 4.6l4.8 4.8M9.4 4.6l-4.8 4.8',
  // Nothing inside: an empty box is what "has not run" looks like.
  pending: null,
};

/** Which of the four is filled rather than stroked. */
const FILLED: MarkState[] = ['current'];

export function StatusMark({ state, label }: { state: MarkState; label?: string }) {
  const inner = MARK_PATHS[state];
  const filled = FILLED.includes(state);
  return (
    <span
      data-ui="status-mark"
      data-state={state}
      className={styles.mark}
      aria-hidden={label ? undefined : true}
      role={label ? 'img' : undefined}
      aria-label={label}
    >
      <svg viewBox="0 0 14 14" fill="none" width="14" height="14">
        <rect
          x=".75"
          y=".75"
          width="12.5"
          height="12.5"
          rx="6"
          stroke="currentColor"
          strokeWidth="1.5"
        />
        {inner ? (
          <path
            d={inner}
            {...(filled
              ? { fill: 'currentColor' }
              : { stroke: 'currentColor', strokeWidth: 1.5, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const })}
          />
        ) : null}
      </svg>
    </span>
  );
}
