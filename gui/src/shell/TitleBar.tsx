import type { ReactNode } from 'react';

import { Chip, Wordmark } from '../ui';
import styles from './TitleBar.module.css';

/**
 * The top strip: who this is, what you are working on, and three quiet actions.
 *
 * It replaces a 54px antd `Header` that carried a five-item tab `Menu` plus four
 * coloured dropdown chips. The tabs are gone because they were the wrong axis --
 * they split one session across five screens, so looking at the serial port lost
 * your chat. Serial and Flash are panes inside the inspector now, Settings is a
 * drawer, and the workbench is a screen you open deliberately.
 *
 * What is left is deliberately thin. The chips that used to live here (mode,
 * thinking level, context usage) moved to the status bar, which is where live
 * session state belongs -- they are readings, not navigation.
 *
 * Parity note: the height is `--h-bar` rather than the ~36px a native title bar
 * gets, because this bar carries the project path and a monospace path needs the
 * room. The window's own frame is still the operating system's; nothing here
 * drags it.
 */
export function TitleBar({
  project,
  git,
  actions,
}: {
  /** The workspace the rail and the agent are pointed at. */
  project: string;
  /** Branch and dirty count, when a project with a repository is open. */
  git?: { branch: string; dirty: number } | null;
  /**
   * The right-hand cluster -- `TitleBarActions` in the app. Composed by the caller
   * so this file owns the frame and not the set of things that happen to be
   * buttons today.
   */
  actions?: ReactNode;
}) {
  return (
    <header data-ui="title-bar" className={styles.bar}>
      <img src="/icons/logo-w-64.png" alt="" aria-hidden className={styles.mark} />
      <Wordmark />

      <span aria-hidden className={styles.divider} />

      {/*
        The project path is the one piece of context worth pinning to the top:
        every tool call, every session and the rail are all relative to it, and it
        used to be a 165px-wide text input in the sidebar with no label.
      */}
      <span title={project} className={styles.project}>
        {project}
      </span>

      {git && (
        <Chip mono title={git.dirty > 0 ? `${git.dirty} changed files` : 'Working tree is clean'}>
          {git.branch}
          {git.dirty > 0 ? ` ·${git.dirty}` : ''}
        </Chip>
      )}

      <span className={styles.spacer} />
      {actions}
    </header>
  );
}
