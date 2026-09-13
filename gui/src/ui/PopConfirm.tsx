import { useId, useRef } from 'react';
import type { ReactNode, RefObject } from 'react';

import { Button } from './Button';
import styles from './PopConfirm.module.css';
import { Popover } from './Popover';
import { useFocusTrap } from './useFocusTrap';
import type { Align, Side } from './types';

export interface PopConfirmProps {
  open: boolean;
  /** The control that asked -- a row's delete button, a session's kebab. */
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  onConfirm: () => void;
  title: ReactNode;
  message?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: 'primary' | 'danger';
  side?: Side;
  align?: Align;
}

/**
 * A `Confirm` that answers where it was asked.
 *
 * The difference is not size, it is context. Deleting a session from a sidebar
 * row asks about *that row* -- a centred modal covering the list makes the user
 * re-read the name to be sure they picked the right one, which is precisely the
 * moment a wrong deletion happens. The panel hangs off the trigger, so the row it
 * is about stays on screen next to the question.
 *
 * Same focus rules as the dialog it mirrors: the trap holds Tab inside the panel,
 * Escape is a cancel rather than an answer-by-ignoring, and focus returns to the
 * trigger when it closes. `danger` again focuses cancel.
 *
 * It is small and anchored rather than wide, so it carries its own padding in
 * `padded` instead of a form layout: two sentences and two buttons is the whole
 * budget, and anything more is a `Modal`.
 */
export function PopConfirm({
  open,
  anchorRef,
  onClose,
  onConfirm,
  title,
  message,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  tone = 'primary',
  side = 'bottom',
  align = 'start',
}: PopConfirmProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const titleId = useId();
  const danger = tone === 'danger';
  useFocusTrap(panelRef, { active: open, onEscape: onClose });

  return (
    <Popover
      open={open}
      anchorRef={anchorRef}
      onClose={onClose}
      panelRef={panelRef}
      padded
      role="dialog"
      labelledBy={titleId}
      side={side}
      align={align}
    >
      {/* The width lives here, not in `Popover`'s `className`: that prop exists for
          the tooltip's pointer rule and nothing else, and a panel two words wide is
          a panel whose buttons are wrapping. */}
      <div className={styles.content}>
        <p id={titleId} className={styles.title}>
          {title}
        </p>
        {message ? <p className={styles.message}>{message}</p> : null}
        <div className={styles.actions}>
          <Button
            tier="ghost"
            size="sm"
            data-autofocus={danger ? 'true' : undefined}
            onClick={onClose}
          >
            {cancelLabel}
          </Button>
          <Button
            tier={danger ? 'danger' : 'primary'}
            size="sm"
            data-autofocus={danger ? undefined : 'true'}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </Popover>
  );
}
