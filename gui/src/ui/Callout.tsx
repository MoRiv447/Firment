import { CircleAlert, Info, TriangleAlert } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { Icon } from './Icon';
import type { CalloutTone } from './types';
import styles from './Callout.module.css';

/** The mark each tone carries, so a callout says what it is before it is read. */
const GLYPHS: Record<CalloutTone, LucideIcon> = {
  info: Info,
  warn: TriangleAlert,
  failed: CircleAlert,
};

/**
 * A block of prose the interface is telling you, not something you typed.
 *
 * It replaces the antd `Alert`, which was doing four jobs in this app: the stall
 * notice, the "dangerous command" warning, the tool-result body, and a stream of
 * runtime notices. Only one of those is a banner, and the alert styling it put on
 * all four is why the transcript looked like an incident log -- a wall of amber
 * and blue boxes where three of them were just text with a fact attached.
 *
 * The tone names the *severity*, and the fill and the ink come from one measured
 * pair (`--warn-bg` with `--warn-ink`, and so on), which is the same rule `Chip`
 * follows and the reason neither can be assembled wrong. Body text is prose, so
 * it keeps the pair's ink rather than falling back to `--muted`: on an amber
 * ground the muted grey is 2.5:1.
 *
 * `title` is optional because a one-line notice with a heading above it is a
 * heading with a sentence under it, and `children` carries the raw tool output
 * where that is what the block is holding.
 */
export function Callout({
  tone = 'info',
  icon = GLYPHS[tone],
  mono = false,
  title,
  children,
}: {
  tone?: CalloutTone;
  /** Pass `null` for a block that should not announce itself with a glyph. */
  icon?: LucideIcon | null;
  /** The body is raw output rather than prose: mono, and its line breaks kept. */
  mono?: boolean;
  title?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div
      data-ui="callout"
      data-tone={tone}
      data-mono={mono || undefined}
      className={styles.root}
    >
      {icon ? (
        <span className={styles.glyph}>
          <Icon src={icon} />
        </span>
      ) : null}
      <div className={styles.text}>
        {title ? <p className={styles.title}>{title}</p> : null}
        {children ? <div className={styles.body}>{children}</div> : null}
      </div>
    </div>
  );
}
