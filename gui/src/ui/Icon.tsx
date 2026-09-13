import type { LucideIcon } from 'lucide-react';

import { cx } from './cx';
import styles from './Icon.module.css';

/**
 * An icon, sized and coloured by the text around it.
 *
 * Two rules, both of which the old tree broke:
 *
 * * **Size is `1em`.** `@ant-design/icons` shipped a 16px box that had to be
 *   told to shrink, so the transcript ended up with `9px` font-sizes written down
 *   purely to make a chevron small enough to sit next to an 11px label. An icon
 *   measured in `em` takes the size of the type step it is in and no call site
 *   needs a number.
 * * **There is no brand tone.** `tokens.md` says the acid green is identity, not
 *   feedback, and it is ~1.8:1 on a light ground -- an acid icon is invisible
 *   there and shouting on dark. So `tone` offers the ink colours and the status
 *   colours, and a call site that wants acid has to go through `Chip` or
 *   `Button`, where the fill and its text were measured as a pair.
 *
 * `className` is allowed here (and in `KeyValue`) because an icon's placement is
 * its parent's business: `gap` and `align-items` are what position it, and a
 * wrapper `<span>` to carry a margin would add a node to every row in the app.
 */
export function Icon({
  src: Src,
  size,
  tone = 'inherit',
  spin = false,
  label,
  className,
}: {
  src: LucideIcon;
  /** `sm` is 1em of the surrounding type; the other two step up from it. */
  size?: 'sm' | 'md' | 'lg';
  tone?:
    | 'inherit'
    | 'muted'
    | 'ink'
    | 'on-selection'
    | 'ok'
    | 'failed'
    | 'running'
    | 'attention';
  /** For a control that is working: a spinner, not a "loading" label. */
  spin?: boolean;
  /**
   * Omit it for a decorative icon, which is most of them: the row already says
   * "Build" in words, and `aria-hidden` is what stops a screen reader reading
   * the same thing twice.
   */
  label?: string;
  className?: string;
}) {
  return (
    <Src
      className={cx(styles.icon, className)}
      data-ui="icon"
      data-size={size}
      data-tone={tone === 'inherit' ? undefined : tone}
      data-spin={spin || undefined}
      aria-hidden={label ? undefined : true}
      role={label ? 'img' : undefined}
      aria-label={label}
    />
  );
}
