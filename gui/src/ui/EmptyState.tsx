import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import styles from './EmptyState.module.css';
import { Icon } from './Icon';

/**
 * The panel that says there is nothing to show, and what would put something here.
 *
 * Every list in this app has an empty moment -- no sessions, no changes, no
 * matches for the filter -- and the old tree handled each one with a different
 * sentence in a different grey, three of them a spinner, one of them a `0`. A
 * `EmptyState` is the one place that says it, and the second line is required by
 * the shape of it: "No results" is a dead end, "No results for 'flash'" tells the
 * user what to type instead.
 *
 * `hint` is the actionable half. When there is an action available, pass it as
 * `action` rather than writing "press Ctrl+N" in prose the user cannot click.
 *
 * No `className`: it fills whatever pane it is given, and a caller that wants a
 * smaller empty message wants a sentence, not this component.
 */
export function EmptyState({
  icon: Glyph,
  title,
  hint,
  action,
}: {
  icon?: LucideIcon;
  title: ReactNode;
  /** One line on why it is empty, or on what would fill it. */
  hint?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div data-ui="empty-state" className={styles.root}>
      {Glyph ? (
        <span className={styles.glyph}>
          <Icon src={Glyph} size="lg" tone="muted" />
        </span>
      ) : null}
      <p className={styles.title}>{title}</p>
      {hint ? <p className={styles.hint}>{hint}</p> : null}
      {action ? <div className={styles.action}>{action}</div> : null}
    </div>
  );
}
