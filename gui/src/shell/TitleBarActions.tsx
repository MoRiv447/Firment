import { LayoutDashboard, Settings, Sun, Moon } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { IconButton, Tooltip, useTooltip } from '../ui';
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
  pressed,
}: {
  label: string;
  icon: LucideIcon;
  onClick: () => void;
  /** Only for a control that stays on: the workbench, not the theme. */
  pressed?: boolean;
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
        aria-pressed={pressed}
      />
      {/* Below, not above: every one of these hangs off the top strip. */}
      <Tooltip tip={tip} text={label} side="bottom" />
    </>
  );
}

/**
 * The right-hand cluster of the title bar.
 *
 * Four controls and nothing else. The three that used to sit next to them -- mode,
 * thinking level, context usage -- are readings rather than commands, so they live
 * in the status bar now, and the fifth (the workbench tab) stopped being a tab the
 * moment it became a screen you open deliberately.
 */
export function TitleBarActions({
  mode,
  onToggleTheme,
  workbenchOpen,
  onToggleWorkbench,
  onOpenSettings,
  notifications,
  unread,
  onMarkAllRead,
  onClear,
  onOpenSession,
}: {
  mode: ThemeMode;
  onToggleTheme: () => void;
  workbenchOpen: boolean;
  onToggleWorkbench: () => void;
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
      <NotificationBell
        notifications={notifications}
        unread={unread}
        onMarkAllRead={onMarkAllRead}
        onClear={onClear}
        onOpenSession={onOpenSession}
      />
      <Action
        label="Project workbench"
        icon={LayoutDashboard}
        pressed={workbenchOpen}
        onClick={onToggleWorkbench}
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
