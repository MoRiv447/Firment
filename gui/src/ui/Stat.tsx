import type { ReactNode } from 'react';

import styles from './Stat.module.css';

/**
 * A number that is the answer, with what it counts underneath it.
 *
 * The pair `KeyValue` cannot say: there the label is the question and the two share
 * a line. Here the value is what the eye lands on -- monospace, because these are
 * counts and sizes compared against the one in the card beside them, and a
 * proportional face makes that comparison a lookup rather than a shape.
 *
 * The label is the quiet half and stays below: a card whose caption is as loud as
 * its number is a card where nothing is the answer.
 */
export function Stat({
  value,
  label,
  hint,
}: {
  value: ReactNode;
  label: ReactNode;
  /** Units, a delta, or what exactly is being counted. */
  hint?: ReactNode;
}) {
  return (
    <div data-ui="stat" className={styles.root}>
      <span className={styles.value}>{value}</span>
      <span className={styles.label}>{label}</span>
      {hint ? <span className={styles.hint}>{hint}</span> : null}
    </div>
  );
}
