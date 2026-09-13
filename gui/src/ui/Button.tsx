import { LoaderCircle } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';

import { cx } from './cx';
import { Icon } from './Icon';
import type { Size, Tier } from './types';
import styles from './Button.module.css';

type NativeButton = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'>;

export interface ButtonProps extends NativeButton {
  /** How much the button announces itself. See `Tier` in `types.ts`. */
  tier?: Tier;
  size?: Size;
  /**
   * The brand's cut corner, on the primary CTA. `'left'` removes the bottom-left
   * triangle; `'both'` leans the whole control forward.
   *
   * Only honoured on `tier="primary"`, because the cut needs a fill of its own to
   * sit on: a hairline box already has an edge, and a second one on top of it is
   * the grey-ring-around-green mistake this token exists to prevent.
   */
  edge?: 'left' | 'both';
  icon?: LucideIcon;
  iconSide?: 'start' | 'end';
  /** Spins in place of the icon and sets `aria-busy`. Not a disabled state. */
  loading?: boolean;
  /** Stretch to the width of the row. */
  full?: boolean;
  children?: ReactNode;
  ref?: Ref<HTMLButtonElement>;
}

/**
 * The button, and the only place the slant is drawn.
 *
 * `ActionButton.tsx` (171 lines) is what this replaces. It tracked hover and
 * focus in React state, so every hover over a tool card re-rendered the card, and
 * it painted its own edge colour by reading `color.outline` -- which is how the
 * CTA's cut corner ended up ringed in grey once `outline` became an ordinary
 * border. Both halves are CSS here (`:hover`, `[data-edge]`), so the component is
 * the markup and the props, and a row of them costs nothing to move the mouse
 * across.
 */
export function Button({
  tier = 'secondary',
  size = 'md',
  edge,
  icon: Leading,
  iconSide = 'start',
  loading = false,
  full = false,
  children,
  className,
  disabled,
  type = 'button',
  ref,
  ...rest
}: ButtonProps) {
  const cut = tier === 'primary' ? edge : undefined;
  const glyph = loading ? (
    <Icon src={LoaderCircle} spin />
  ) : Leading ? (
    <Icon src={Leading} />
  ) : null;

  return (
    <button
      {...rest}
      ref={ref}
      type={type}
      disabled={disabled}
      aria-busy={loading || undefined}
      data-ui="button"
      data-tier={tier}
      data-size={size}
      data-edge={cut}
      data-full={full || undefined}
      data-icon-only={!children && !!Leading ? 'true' : undefined}
      className={cx(styles.root, className)}
    >
      <span className={styles.fill}>
        {iconSide === 'start' && glyph}
        {children ? <span className={styles.label}>{children}</span> : null}
        {iconSide === 'end' && glyph}
      </span>
    </button>
  );
}

/**
 * A button with no words.
 *
 * `label` is required rather than optional: an icon-only control with no
 * accessible name is how a toolbar becomes a guessing game, and the same string
 * is what a `Tooltip` needs, so there is no reason to leave it out. It renders
 * `data-ui="button"` with `data-icon-only`, which is the honest description --
 * this is the same control, with the label moved from the text to the name.
 */
export function IconButton({
  label,
  icon,
  tier = 'ghost',
  ...rest
}: Omit<ButtonProps, 'children' | 'icon' | 'iconSide'> & {
  label: string;
  icon: LucideIcon;
}) {
  return <Button {...rest} icon={icon} tier={tier} aria-label={label} />;
}
