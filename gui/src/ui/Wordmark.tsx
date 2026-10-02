import styles from './Wordmark.module.css';

/**
 * The name, in the two sizes it appears at.
 *
 * It exists because the old tree had the same word set twice -- 28px/800 in the
 * chat empty state and 14px/600 in the title bar -- with neither one defined
 * anywhere, which is how a wordmark ends up in three weights and two greys. One
 * component, two steps, and the display step is the only place `--fs-display` is
 * allowed to appear.
 *
 * The trailing period is the mark's own gesture in the other half of the identity:
 * `LogoMark` draws the three bars, this sets the name, and the acid dot is the one
 * pixel of colour either of them carries. It is `aria-hidden` because a screen
 * reader should say the name, not the punctuation.
 */
export function Wordmark({ size = 'sm' }: { size?: 'sm' | 'lg' }) {
  return (
    <span data-ui="wordmark" data-size={size} className={styles.wordmark}>
      Firment
      <span aria-hidden className={styles.dot}>
        .
      </span>
    </span>
  );
}
