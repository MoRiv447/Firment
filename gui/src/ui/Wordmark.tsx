import styles from './Wordmark.module.css';

/**
 * The name, as the title bar draws it.
 *
 * It exists because the old tree had the same word set twice -- 28px/800 in the
 * chat empty state and 14px/600 in the title bar -- with neither one defined
 * anywhere, which is how a wordmark ends up in three weights and two greys. One
 * component, one step. The 28px display step left with the empty state that used
 * it: a `size` prop with no caller is not a size, it is a second definition
 * waiting to disagree with the first, and the gallery's own type sample
 * (`dev/Showcase.module.css`) is where `--fs-display` is shown now.
 *
 * The trailing period is the mark's own gesture in the other half of the identity:
 * `LogoMark` draws the three bars, this sets the name, and the acid dot is the one
 * pixel of colour either of them carries. It is `aria-hidden` because a screen
 * reader should say the name, not the punctuation.
 */
export function Wordmark() {
  return (
    <span data-ui="wordmark" className={styles.wordmark}>
      Firment
      <span aria-hidden className={styles.dot}>
        .
      </span>
    </span>
  );
}
