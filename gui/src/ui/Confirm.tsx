import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';

import { Button } from './Button';
import styles from './Confirm.module.css';
import { Modal } from './Modal';

export interface ConfirmProps {
  open: boolean;
  title: ReactNode;
  /** One sentence about what is about to happen. Omit for a bare yes/no. */
  message?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  /** `danger` for anything the user cannot undo afterwards. */
  tone?: 'primary' | 'danger';
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * The two-button question: "are you sure", answered before anything happens.
 *
 * A `Modal` with a fixed footer, because the alternative is every view writing
 * its own delete dialogue -- which is what the old tree had, in three widths and
 * with the destructive action on the left in one place and the right in another.
 *
 * Which button gets focus is the one real decision here. A normal confirm focuses
 * the affirmative action, because "Confirm" is what the user was trying to do.
 * A `danger` one focuses *cancel*: the whole point of a destructive confirm is
 * that an accidental Enter must not destroy anything, and a dialog that puts the
 * caret on Delete has asked the question and answered it at the same time.
 */
export function Confirm({
  open,
  title,
  message,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  tone = 'primary',
  onConfirm,
  onCancel,
}: ConfirmProps) {
  const danger = tone === 'danger';
  return (
    <Modal
      open={open}
      title={title}
      onClose={onCancel}
      size="sm"
      footer={
        <>
          <Button
            tier="ghost"
            data-autofocus={danger ? 'true' : undefined}
            onClick={onCancel}
          >
            {cancelLabel}
          </Button>
          <Button
            tier={danger ? 'danger' : 'primary'}
            data-autofocus={danger ? undefined : 'true'}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      {message ? <p className={styles.message}>{message}</p> : null}
    </Modal>
  );
}

/** What `confirm()` takes -- everything except the state a promise already carries. */
export type ConfirmOptions = Omit<ConfirmProps, 'open' | 'onConfirm' | 'onCancel'>;

/**
 * The same question as a promise, for a handler that is already `async`.
 *
 * The imperative half exists for `WorkbenchView.tsx:581`, where the answer arrives
 * in the middle of a `try`/`catch` around an API call. Written with `useState` that
 * block needs an open flag, a pending-file ref to remember what was being
 * reloaded, and an effect to run the continuation -- three pieces of state to hold
 * one boolean. `if (await confirm({ ... }))` is the same thing in one line.
 *
 * It mounts its own root into a detached `<div>` rather than reaching into the
 * app's: the panel portals to `overlayRoot()` like every other dialog, so the host
 * node never has layout, and `Modal` consumes no context -- there is nothing in
 * `#root` it needs to inherit.
 *
 * Shadows `window.confirm` at the call site on purpose. It cannot take a raw
 * string, and a call site that passes one is a type error rather than a
 * browser-native dialog with no theme and no way to say "Reload".
 */
export function confirm(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    let settled = false;
    const settle = (value: boolean) => {
      if (settled) return;
      settled = true;
      resolve(value);
      // Deferred: this runs from inside a click handler, and unmounting the tree
      // that is still dispatching is not allowed. The focus trap's cleanup --
      // which puts the caret back where it started -- has to run against a live
      // tree for that to mean anything.
      queueMicrotask(() => {
        root.unmount();
        host.remove();
      });
    };
    root.render(
      <Confirm
        {...options}
        open
        onConfirm={() => settle(true)}
        onCancel={() => settle(false)}
      />,
    );
  });
}
