import styles from './LogoMark.module.css';

/**
 * The mark, drawn rather than loaded.
 *
 * It used to be `<img src="/icons/logo-w-64.png">` in the title bar. That file is
 * the *light-ground* cut of the logo — a dark-green glyph meant for a pale
 * background — sitting on `--surface`, which in the default dark scheme is nearly
 * black, so the one piece of brand identity in the window was almost invisible
 * and got worse every time the palette moved.
 *
 * Three skewed bars are the same shape the asset is cut from. They follow
 * `--brand-acid`, so both schemes get the right value for free, and there is no
 * second file to keep in step with the palette.
 */
export function LogoMark() {
  return (
    <span aria-hidden data-ui="logo-mark" className={styles.mark}>
      <span className={styles.barTop} />
      <span className={styles.barMid} />
      <span className={styles.barLow} />
    </span>
  );
}
