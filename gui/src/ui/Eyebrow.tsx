import type { ReactNode } from 'react';

import styles from './Eyebrow.module.css';

/**
 * A micro-label, with a short brand rule before it.
 *
 * "Sessions", "One requirement, end to end" -- the small uppercase line that names
 * a region or a moment without becoming a heading. It is a primitive because the
 * *rule before it* is the part that carries meaning: a 12px bar in the brand green
 * is what makes a 10px tracked line read as a label rather than as leftover text,
 * and a call site that drew its own would be a call site that could quietly drop it.
 *
 * **Tracking is per language and the call site has to say which.** `--tracking-label`
 * is 0.16em, which is designed for uppercase Latin: narrow glyphs that open up well.
 * CJK glyphs are already full-width, so the same number reads as a line of separated
 * characters. `latin` is that switch, and it defaults to the CJK value because
 * guessing "looks like Latin" from a string is exactly the kind of cleverness that
 * fails on the first mixed-language label.
 *
 * `upper` is the other half and is separate from `latin`: a Chinese label is not
 * uppercased (it has no case) and should not be asked to be, but a Latin one that
 * is not uppercased is a different, quieter thing -- so the two are independent
 * flags rather than one "label" mode.
 */
export function Eyebrow({
  children,
  latin = false,
  upper = true,
}: {
  children: ReactNode;
  /** True when the text is Latin: widens the tracking to the Latin value. */
  latin?: boolean;
  /** False for a label that has no case to change. */
  upper?: boolean;
}) {
  return (
    <span
      data-ui="eyebrow"
      data-latin={latin || undefined}
      data-upper={upper || undefined}
      className={styles.eyebrow}
    >
      {children}
    </span>
  );
}
