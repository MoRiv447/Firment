import { Settings, Sun, Moon } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { IconButton, StatusDot, Tooltip, useTooltip } from '../ui';
import { NotificationBell } from './NotificationBell';
import styles from './TitleBarActions.module.css';
import type { ThemeMode } from '../lib/theme';
import type { NotificationEntry } from '../types';

/**
 * One control, named by the string a screen reader says and by the same string a
 * tooltip shows.
 *
 * `useTooltip` is per-control and cannot be hoisted into the parent: a tooltip has
 * to sit on the element it describes, which is why the primitive is a hook plus a
 * panel rather than a wrapper. So the wrapper is here instead, and it is a
 * component rather than a `<span>`, so nothing extra lands in the flex row.
 */
function Action({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon: LucideIcon;
  onClick: () => void;
}) {
  const tip = useTooltip<HTMLButtonElement>();
  return (
    <>
      <IconButton
        ref={tip.anchorRef}
        {...tip.triggerProps}
        label={label}
        icon={icon}
        onClick={onClick}
      />
      {/* Below, not above: every one of these hangs off the top strip. */}
      <Tooltip tip={tip} text={label} side="bottom" />
    </>
  );
}

/**
 * The right-hand cluster of the title bar.
 *
 * Four controls, and one reading that is only sometimes there.
 *
 * The three readings that used to sit here -- mode, thinking level, context usage --
 * moved to the status bar and stay there. **The pill is not one of them**, and the
 * difference is the reason it is allowed to live here: those three are *permanent*,
 * so they belong with the other permanent readings at the bottom; this one exists
 * only while a turn is in flight and disappears when it ends. A transient state
 * belongs near the identity of the window, which is where the eye already is when
 * something starts happening.
 *
 * It is a reading and not a control: nothing to press, no hover, `aria-live` off --
 * a status that announced itself every second would be a screen reader reading a
 * stopwatch.
 */
export function TitleBarActions({
  mode,
  running,
  onToggleTheme,
  onOpenSettings,
  notifications,
  unread,
  onMarkAllRead,
  onClear,
  onOpenSession,
}: {
  mode: ThemeMode;
  /**
   * The turn in flight, when there is one. Absent is the normal case: a window
   * sitting idle has nothing to report here, and the pill is rendered only for the
   * stretch where that is false.
   *
   * `current` / `total` are the workflow's own step count -- the same numbers the
   * step row under the transcript shows -- and they are optional because a turn
   * that has not touched a workflow tool has no denominator to offer.
   */
  running?: { tool: string; seconds: number; current?: number; total?: number } | null;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  notifications: NotificationEntry[];
  unread: number;
  onMarkAllRead: () => void;
  onClear: () => void;
  onOpenSession: (sid: string) => void;
}) {
  const themeLabel =
    mode === 'dark' ? 'Switch to the light scheme' : 'Switch to the dark scheme';

  return (
    <div className={styles.cluster}>
      {running && (
        <span className={styles.pill}>
          <StatusDot status="running" pulse />
          <span className={styles.pillTool}>{running.tool}</span>
          <span aria-hidden className={styles.sep} />
          {running.total ? (
            <>
              <span className={styles.pillCount}>
                {running.current} / {running.total}
              </span>
              <span aria-hidden className={styles.sep} />
            </>
          ) : null}
          <span className={styles.pillTime}>{running.seconds}s</span>
        </span>
      )}
      <NotificationBell
        notifications={notifications}
        unread={unread}
        onMarkAllRead={onMarkAllRead}
        onClear={onClear}
        onOpenSession={onOpenSession}
      />
      <Action label="Settings" icon={Settings} onClick={onOpenSettings} />
      <Action
        label={themeLabel}
        icon={mode === 'dark' ? Sun : Moon}
        onClick={onToggleTheme}
      />
    </div>
  );
}
