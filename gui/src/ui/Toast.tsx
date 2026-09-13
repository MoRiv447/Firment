import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { Icon } from './Icon';
import styles from './Toast.module.css';
import { dismissToast, useToasts, type ToastTone } from './toastStore';

const TONE_ICON: Record<ToastTone, LucideIcon> = {
  info: Info,
  ok: CircleCheck,
  failed: CircleAlert,
  attention: TriangleAlert,
};

/**
 * The stack of toasts, mounted once in the shell.
 *
 * Bottom-right, above the status bar, because that corner is the only part of this
 * window that never holds something the user is reading: the transcript is on the
 * left, the inspector on the right, the composer at the bottom centre.
 *
 * The stack is one `aria-live="polite"` region, so a toast is read when it lands
 * without stealing the caret. The region sits on the container rather than on each
 * panel on purpose -- a live element added at the same moment as its own text is a
 * live element whose text may never be announced.
 */
export function ToastViewport() {
  const toasts = useToasts();

  // Always mounted, even when empty: the live region has to exist before the toast
  // that is announced in it.
  return (
    <div data-ui="toast-stack" aria-live="polite" className={styles.stack}>
      {toasts.map((toast) => (
        <div key={toast.id} data-ui="toast" data-tone={toast.tone} className={styles.toast}>
          <Icon src={TONE_ICON[toast.tone]} className={styles.glyph} />
          <span className={styles.text}>{toast.text}</span>
          <button
            type="button"
            aria-label="Dismiss"
            className={styles.dismiss}
            onClick={() => dismissToast(toast.id)}
          >
            <Icon src={X} tone="muted" />
          </button>
        </div>
      ))}
    </div>
  );
}
