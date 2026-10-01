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
 * * **`tone` offers the ink colours, the status colours, and a brand tone that
 *   only became usable when the palette changed.** The rule used to be "there is
 *   no brand tone", on the measurement that the old acid was ~1.8:1 on a light
 *   ground -- invisible there and shouting on dark -- so a call site that wanted
 *   acid went through `Chip` or `Button`, where a fill and its text were measured
 *   as a pair.
 *
 *   That measurement belongs to the palette this replaced. The brand has two
 *   values now and the text one is `--brand-ink`: 14.82:1 on the dark surface and
 *   5.48:1 on the light one. Both are comfortable for a 1.75px stroke, so `brand`
 *   resolves to `--brand-ink` rather than to the fill, and the old rule is written
 *   down as superseded instead of quietly disappearing.
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
    | 'brand'
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
