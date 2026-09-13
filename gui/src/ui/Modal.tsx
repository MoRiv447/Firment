import { X } from 'lucide-react';
import { useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import type { ReactNode, RefObject } from 'react';

import { cx } from './cx';
import styles from './dialog.module.css';
import { Icon } from './Icon';
import { Scrim } from './Overlay';
import { overlayRoot } from './portal';
import { useFocusTrap } from './useFocusTrap';

interface SheetProps {
  panelRef: RefObject<HTMLDivElement | null>;
  titleId: string;
  title: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  footer?: ReactNode;
  ui: 'modal' | 'drawer';
  className: string;
  /** False for a dialog whose answer is required: no × and no click-past. */
  dismissable: boolean;
  size: string;
}

/**
 * The panel both kinds of dialog are made of.
 *
 * Private on purpose. `Modal` and `Drawer` are one sheet with two positions, and
 * the moment that markup exists twice the two drift apart -- which is exactly what
 * happened between antd's `Modal` and the hand-rolled settings drawer this
 * replaces: different heading size, different body padding, two dialects of
 * "cancel".
 */
function Sheet({
  panelRef,
  titleId,
  title,
  onClose,
  children,
  footer,
  ui,
  className,
  dismissable,
  size,
}: SheetProps) {
  return createPortal(
    <>
      <Scrim onDismiss={dismissable ? onClose : undefined} />
      <div
        ref={panelRef}
        data-ui={ui}
        data-size={size}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={className}
      >
        <header className={styles.header}>
          <h2 id={titleId} className={styles.title}>
            {title}
          </h2>
          {dismissable ? (
            <button
              type="button"
              aria-label="Close"
              onClick={onClose}
              className={styles.close}
            >
              <Icon src={X} tone="muted" />
            </button>
          ) : null}
        </header>
        {/* An empty body would still carry its own padding, so a title-and-buttons
            dialog would open with a visible gap where the content should be. */}
        {children === null || children === undefined || children === false ? null : (
          <div className={styles.body}>{children}</div>
        )}
        {footer ? <footer className={styles.footer}>{footer}</footer> : null}
      </div>
    </>,
    overlayRoot(),
  );
}

/**
 * A window that demands an answer before the app continues.
 *
 * Three things are non-negotiable here, and they are the reason this is a
 * primitive rather than a div: the rest of the app goes `inert` (see `portal.ts`),
 * Tab wraps inside the panel, and focus returns to whatever opened it.
 * `role="dialog"` with `aria-modal` is what keeps a screen reader in the window,
 * and `aria-labelledby` points at the heading rather than repeating it as a label.
 */
export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  size = 'sm',
  dismissable = true,
}: {
  open: boolean;
  /** Rendered as the heading and used as the dialog's accessible name. */
  title: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  /** The action row. Omit it for an informational panel with only a close. */
  footer?: ReactNode;
  /** How much room to ask for: `lg` is for a diff, `sm` for a question. */
  size?: 'sm' | 'md' | 'lg';
  /**
   * False for a dialog whose answer is required -- the permission prompt is the
   * case. Clicking past it, or pressing Escape, must not count as an answer.
   */
  dismissable?: boolean;
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const titleId = useId();
  useFocusTrap(panelRef, { active: open, onEscape: dismissable ? onClose : undefined });

  if (!open) return null;

  return (
    <Sheet
      panelRef={panelRef}
      titleId={titleId}
      title={title}
      onClose={onClose}
      footer={footer}
      dismissable={dismissable}
      size={size}
      ui="modal"
      className={cx(styles.sheet, styles.modal)}
    >
      {children}
    </Sheet>
  );
}

/**
 * A panel that arrives from the side and stays until it is closed.
 *
 * The settings screen, the serial monitor and the flash log all want the same
 * thing: a surface wide enough for a form, with the transcript still visible
 * alongside it so the user can see what the settings are for. A `Modal` covers the
 * whole app and loses that context; a pane inside the grid steals width from its
 * neighbour and cannot scroll on its own.
 *
 * Same sheet and the same rules as `Modal`, so opening a drawer and opening a
 * dialog feel like two sizes of one thing rather than two systems.
 */
export function Drawer({
  open,
  title,
  onClose,
  children,
  footer,
  size = 'md',
}: {
  open: boolean;
  title: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  /** Sticky at the bottom of the panel, above the fold whatever the body holds. */
  footer?: ReactNode;
  size?: 'md' | 'lg';
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const titleId = useId();
  useFocusTrap(panelRef, { active: open, onEscape: onClose });

  if (!open) return null;

  return (
    <Sheet
      panelRef={panelRef}
      titleId={titleId}
      title={title}
      onClose={onClose}
      footer={footer}
      dismissable
      size={size}
      ui="drawer"
      className={cx(styles.sheet, styles.drawer)}
    >
      {children}
    </Sheet>
  );
}
