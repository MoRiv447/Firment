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
 * The logo artwork is a separate thing (an `<img>` in the title bar) and keeps
 * its own cut corner: the slant in the mark is drawn into the file, so there is
 * nothing to reproduce here.
 */
export function Wordmark({ size = 'sm' }: { size?: 'sm' | 'lg' }) {
  return (
    <span data-ui="wordmark" data-size={size} className={styles.wordmark}>
      Firment
    </span>
  );
}
