import type { ReactNode } from 'react';

import styles from './Card.module.css';

/**
 * A titled region that owns one subject.
 *
 * The workbench is ten `Card type="inner" size="small"` panels, and this is what
 * replaces that idiom. `extra` keeps antd's name for the slot because it is the
 * one it had right: the single thing that qualifies the whole card -- what it is
 * for, a count, the control that refreshes it. Not a stack of buttons; a card
 * whose header is a toolbar is a toolbar with a label on it.
 *
 * `size`, `bordered`, `headStyle`, `bodyStyle` and `actions` are not carried over.
 * The first two chose between paddings the token layer now decides, and the last
 * three exist so a call site can re-style the chrome from the outside -- which is
 * how a view ends up with ten slightly different boxes instead of one card.
 */
export function Card({
  title,
  extra,
  children,
}: {
  title: ReactNode;
  extra?: ReactNode;
  children?: ReactNode;
}) {
  const hasBody = children !== null && children !== undefined && children !== false;
  return (
    <section data-ui="card" className={styles.card}>
      <header className={styles.head}>
        <h3 className={styles.title}>{title}</h3>
        {extra ? <span className={styles.extra}>{extra}</span> : null}
      </header>
      {hasBody ? <div className={styles.body}>{children}</div> : null}
    </section>
  );
}
