import styles from './Skeleton.module.css';

/**
 * A placeholder for content that has not arrived yet.
 *
 * Two props, both of them about shape rather than size: `lines` how many bars, and
 * `last` whether the final one is short. The widths are percentages the stylesheet
 * owns, because a skeleton that measures its caller's text is a second layout
 * engine -- and the point of the shape is that it is *not* the real content, it is
 * the outline of where the real content will be.
 *
 * Not a spinner. A spinner says "this is working"; a skeleton says "this has a
 * shape and it is coming", which is the honest thing to draw for a pane that is
 * loading a list. The startup splash keeps its spinner because there the work is a
 * fixed sequence with no shape yet.
 *
 * `aria-hidden`: bars that describe no content have nothing to announce, and a
 * screen reader reading an empty group is noise on top of the wait.
 */
export function Skeleton({
  lines = 1,
  last = 'short',
}: {
  lines?: number;
  /** `short` keeps a stack of bars from reading as a column of numbers. */
  last?: 'short' | 'full';
}) {
  return (
    <div data-ui="skeleton" aria-hidden="true" className={styles.root}>
      {Array.from({ length: lines }, (_, i) => (
        <span
          key={i}
          className={styles.bar}
          data-last={last === 'short' && i === lines - 1 ? 'true' : undefined}
        />
      ))}
    </div>
  );
}
