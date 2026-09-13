import type { ReactNode } from 'react';

import { cx } from './cx';
import styles from './KeyValue.module.css';

/**
 * One labelled fact: a port, a commit sha, a flash size.
 *
 * A grid rather than `label: value` in a sentence, because these come in fours and
 * a reader scans the value column -- which is the half they actually want. The key
 * column is `max-content` so a card with four rows of them lines up on its own
 * without anybody writing a width, and the value column is `minmax(0, 1fr)` so a
 * 40-character path wraps instead of widening the pane. That second half is the
 * reason this is a primitive: the old tree had `word-break` written at four call
 * sites and missing at three more, so the same path broke one card and not the
 * other.
 *
 * Plain `<div>`s rather than a `<dl>`: a definition list has to hold all of its
 * terms in one element, and these are rendered one at a time by components that do
 * not know they are neighbours.
 */
export function KeyValue({
  label,
  value,
  mono = false,
  className,
}: {
  label: ReactNode;
  value: ReactNode;
  /** A path, a sha, a number with units -- anything compared character by character. */
  mono?: boolean;
  /** Placement is the parent's business, as in `Icon`. */
  className?: string;
}) {
  return (
    <div data-ui="key-value" className={cx(styles.root, className)}>
      <span className={styles.key}>{label}</span>
      <span className={styles.value} data-mono={mono || undefined}>
        {value}
      </span>
    </div>
  );
}
