import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Bell, Inbox } from 'lucide-react';

import { Button, EmptyState, IconButton, Popover, StatusDot } from '../ui';
import type { ChipStatus } from '../ui';
import styles from './NotificationBell.module.css';
import type { NotificationEntry } from '../types';

/**
 * Which of the five states a notification is reporting.
 *
 * The mapping lives here rather than at the call site because a guard alert and a
 * failed flash must not drift into two different reds -- `[data-status]` in
 * `StatusDot.module.css` resolves both to the removed-diff ink, and this table is
 * the only thing that decides which of them a `kind` is.
 *
 * An unrecognised `kind` keeps its own name in the panel. The kinds come from the
 * Rust side, so a label the UI has never met is a bug worth being able to read.
 */
const KINDS: Record<string, { label: string; status: ChipStatus }> = {
  guard: { label: 'Guard', status: 'failed' },
  'build-fail': { label: 'Build failed', status: 'failed' },
  'verify-fail': { label: 'Verify failed', status: 'failed' },
  'flash-fail': { label: 'Flash failed', status: 'failed' },
  'device-offline': { label: 'Device offline', status: 'neutral' },
};

const FALLBACK = { status: 'running' as ChipStatus };

/** Today at a clock time; anything older carries its date in front of it. */
function stamp(ts: number): string {
  const at = new Date(ts);
  const time = at.toLocaleTimeString();
  return new Date().toDateString() === at.toDateString() ? time : `${at.toLocaleDateString()} ${time}`;
}

/**
 * The bell and its panel.
 *
 * It takes a list and three callbacks, and it owns the one piece of state that is
 * purely about itself: whether the panel is up. That is not shell logic, and the
 * 90 lines of JSX it replaces sat in the middle of `App.tsx`'s render, which is
 * part of how that file reached a thousand lines.
 *
 * Three things the antd `Popover` it replaces did not do:
 *
 * * A row that names a session is a `<button>`; one that does not is a `<div>`. The
 *   old markup put an `onClick` on a `<span>` for both, which is a control the
 *   keyboard cannot reach -- and a keyboard user is the reader most likely to be
 *   looking at a guard alert.
 * * Focus moves into the panel when it opens. The panel is portalled to the end of
 *   the document, so without this the Tab after opening goes to the next toolbar
 *   button and the panel's own controls are behind the user.
 * * Escape closes it and hands focus back to the bell.
 */
export function NotificationBell({
  notifications,
  unread,
  onMarkAllRead,
  onClear,
  onOpenSession,
}: {
  notifications: NotificationEntry[];
  unread: number;
  onMarkAllRead: () => void;
  onClear: () => void;
  /** Jump to the session a notification came from. */
  onOpenSession: (sid: string) => void;
}) {
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (open) panelRef.current?.focus({ preventScroll: true });
  }, [open]);

  const close = () => {
    setOpen(false);
    anchorRef.current?.focus({ preventScroll: true });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape') return;
    // Stopped, not prevented: nothing above a notifications panel needs to hear
    // that the panel is going away.
    event.stopPropagation();
    close();
  };

  return (
    <span className={styles.bell}>
      <IconButton
        ref={anchorRef}
        label={unread > 0 ? `Notifications, ${unread} unread` : 'Notifications'}
        icon={Bell}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      />
      {unread > 0 && (
        <span aria-hidden className={styles.count}>
          {unread > 99 ? '99+' : unread}
        </span>
      )}

      <Popover
        open={open}
        anchorRef={anchorRef}
        onClose={() => setOpen(false)}
        side="bottom"
        align="end"
        padded
        className={styles.panel}
      >
        <div
          ref={panelRef}
          data-ui="notifications-panel"
          role="region"
          aria-label="Notifications"
          tabIndex={-1}
          onKeyDown={onKeyDown}
        >
          <div className={styles.head}>
            <Button tier="ghost" size="sm" disabled={unread === 0} onClick={onMarkAllRead}>
              Mark all read
            </Button>
            <Button tier="ghost" size="sm" onClick={onClear}>
              Clear
            </Button>
          </div>

          {notifications.length === 0 ? (
            <EmptyState
              icon={Inbox}
              title="No notifications yet"
              hint="Guard alerts, build, verify and flash failures, and offline devices land here."
            />
          ) : (
            <ul className={styles.list}>
              {notifications.map((n) => {
                const kind = KINDS[n.kind];
                const meta = kind ?? { label: n.kind, ...FALLBACK };
                const sid = n.sid;
                const content = (
                  <>
                    <span className={styles.mark}>
                      <StatusDot status={meta.status} />
                    </span>
                    <span className={styles.text}>
                      <span className={styles.title}>{n.title}</span>
                      <span className={styles.meta}>
                        {meta.label} · {stamp(n.ts)}
                      </span>
                      <span className={styles.body}>{n.body}</span>
                    </span>
                  </>
                );
                return (
                  <li key={n.id} className={styles.item}>
                    {sid ? (
                      <button
                        type="button"
                        data-jumpable="true"
                        className={styles.row}
                        onClick={() => {
                          setOpen(false);
                          onOpenSession(sid);
                        }}
                      >
                        {content}
                      </button>
                    ) : (
                      <div className={styles.row}>{content}</div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </Popover>
    </span>
  );
}
