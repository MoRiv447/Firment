import { cx } from './cx';
import styles from './Overlay.module.css';

/**
 * The backdrop a modal panel sits on.
 *
 * Shared by `Modal`, `Drawer` and `Confirm` so there is one answer to "how much
 * of the app do you wash out when something demands an answer" -- the old tree
 * had antd's `Modal` mask in one place and a hand-rolled rgba in another.
 *
 * It is `aria-hidden`, not a `<dialog>`: the panel above it carries the role and
 * the label, and a button-shaped backdrop would be announced as a second control
 * that does nothing but close.
 */
export function Scrim({
  onDismiss,
  className,
}: {
  /** Omit for a scrim that only blocks -- a non-dismissable permission dialog. */
  onDismiss?: () => void;
  className?: string;
}) {
  return (
    <div
      data-ui="scrim"
      aria-hidden="true"
      className={cx(styles.scrim, className)}
      onPointerDown={onDismiss}
    />
  );
}
