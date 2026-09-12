import type { ReactNode } from 'react';
import { font, radius, color } from '../styles/tokens';

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
 * Parity note: the height is 44px rather than Qoder's ~36px because this bar
 * carries the project path, and a monospace path at 11px needs the room.
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
  /** The right-hand cluster. Composed by the caller so this file owns the
   *  frame and not the set of things that happen to be buttons today. */
  actions?: ReactNode;
}) {
  return (
    <header
      style={{
        height: 44,
        flex: '0 0 auto',
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '0 12px',
        background: color.surface,
        borderBottom: `1px solid ${color.line}`,
        fontFamily: font.sans,
      }}
    >
      <img
        src="/icons/logo-w-64.png"
        alt=""
        aria-hidden
        style={{ width: 22, height: 22, borderRadius: radius.chip, objectFit: 'contain' }}
      />
      <span
        style={{
          fontSize: 13,
          fontWeight: 800,
          letterSpacing: 0.6,
          color: color.ink,
          textTransform: 'uppercase',
        }}
      >
        Firment
      </span>

      <span aria-hidden style={{ width: 1, height: 18, background: color.line }} />

      {/*
        The project path is the one piece of context worth pinning to the top:
        every tool call, every session and the rail are all relative to it, and
        it used to be a 165px-wide text input in the sidebar with no label.
        `direction: rtl` keeps the tail of a long path visible -- the leaf is the
        part you recognise -- without a JS truncation.
      */}
      <span
        title={project}
        style={{
          fontSize: 12,
          fontFamily: font.mono,
          color: color.muted,
          maxWidth: 420,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          direction: 'rtl',
          textAlign: 'left',
        }}
      >
        {project}
      </span>

      {git && (
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            fontSize: 11,
            fontFamily: font.mono,
            color: color.muted,
            padding: '2px 8px',
            borderRadius: radius.chip,
            background: color.surfaceRaised,
          }}
        >
          {git.branch}
          {git.dirty > 0 && <span style={{ color: color.warnInk }}>•{git.dirty}</span>}
        </span>
      )}

      <span style={{ flex: 1 }} />
      {actions}
    </header>
  );
}
