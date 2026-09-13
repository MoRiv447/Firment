import type { ChipStatus } from './types';
import styles from './StatusDot.module.css';

/**
 * A state mark with no fill.
 *
 * `Chip` carries the same five states as a coloured pair, and everywhere a status
 * is a *word* that is the right shape. But the status bar, a subagent row and a
 * notification row all say the state in words already -- `Mode agent`, `running`,
 * `build-fail` -- and a second chip there is a badge competing with its own
 * label. The dot is the same vocabulary with the fill taken out: colour only on
 * the mark, so a bar with six readings is not six alerts.
 *
 * `[data-status]` picks the ink rather than a prop carrying a colour, for the
 * same reason `Chip` does it: an ink token is bright on the dark scheme and dark
 * on the light one, so a call site that names a colour is a call site that only
 * works in one scheme.
 *
 * It is `aria-hidden` always. A dot cannot be read; the text next to it is the
 * announcement, and a screen reader that said "ok" twice would be inventing a
 * second fact.
 */
export function StatusDot({
  status,
  pulse = false,
}: {
  status: ChipStatus;
  /** For a state that is live *right now*: running, not merely "in progress" as a category. */
  pulse?: boolean;
}) {
  return (
    <span
      data-ui="status-dot"
      data-status={status}
      data-pulse={pulse || undefined}
      aria-hidden
      className={styles.dot}
    />
  );
}
