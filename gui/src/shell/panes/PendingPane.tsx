import styles from './PendingPane.module.css';

/**
 * A pane that is not built yet, saying so.
 *
 * Three of the inspector's four tabs are honest placeholders: they name what will
 * be here and why it is not yet, rather than rendering an empty box that looks
 * broken. A pane that claims to be a feature and shows nothing is worse than one
 * that admits it is not built.
 *
 * Goes away with the real Changes pane; nothing else should reach for it.
 */
export function PendingPane({ title, body }: { title: string; body: string }) {
  return (
    <div className={styles.root}>
      <p className={styles.title}>{title}</p>
      <p className={styles.body}>{body}</p>
    </div>
  );
}
