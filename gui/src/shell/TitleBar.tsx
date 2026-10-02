import type { ReactNode } from 'react';

import { LogoMark, Wordmark } from '../ui';
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
 * The two rules between the three identity groups are the point of the layout: the
 * project, the session and the brand are three different scopes, and without a
 * rule each side of them reads as one long path.
 */
export function TitleBar({
  project,
  session,
  actions,
}: {
  /** The workspace the rail and the agent are pointed at. */
  project: string;
  /** The open session's name, when one is open. */
  session?: string | null;
  /**
   * The right-hand cluster -- `TitleBarActions` in the app. Composed by the caller
   * so this file owns the frame and not the set of things that happen to be
   * buttons today.
   */
  actions?: ReactNode;
}) {
  return (
    <header data-ui="title-bar" className={styles.bar}>
      <LogoMark />
      <Wordmark />

      <span aria-hidden className={styles.divider} />

      {/*
        The open session's project path: every tool call and every session is
        relative to it, so it is the one piece of context worth pinning to the top.
        The rail still has a field of its own, and it is a different value -- where
        the *next* session starts -- which is why it kept a field rather than
        becoming a read-out like this one.
      */}
      <span title={project} className={styles.project}>
        {project}
      </span>

      {session && (
        <>
          <span aria-hidden className={styles.divider} />
          <span title={session} className={styles.session}>
            {session}
          </span>
        </>
      )}

      <span className={styles.spacer} />
      {actions}
    </header>
  );
}
