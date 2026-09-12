import type { ReactNode } from 'react';
import { Button } from 'antd';

/**
 * The three button weights, delegated to antd.
 *
 * This replaces `SlantButton`, which hand-drew its own chrome: two absolutely
 * positioned `clip-path` layers behind the label, an outer one filled with
 * `color.outline` so the diagonal had an edge a `border` cannot draw on a
 * clipped element, and `border: 1px solid transparent` on the button itself.
 *
 * That was correct while the visual layer was neo-brutalist and the outer layer
 * was pure black. It stopped being correct the moment `outline` became an
 * ordinary border grey: the acid fill kept its dark edge, so a primary button
 * rendered as a green rectangle inside a grey ring -- a fill that looks like it
 * has been outlined by mistake, which is exactly how it read.
 *
 * Nothing here is hand-drawn any more. antd already owns hover, active, focus,
 * disabled and loading for its own buttons, and `antdTheme` already maps
 * `colorPrimary` to `brandAcid` and `Button.primaryColor` to `onAcid`, so the
 * tiers are a `type` mapping rather than a rendering of their own.
 *
 * Weight is still carried by **colour, never by size**: all three tiers render
 * at antd's control height, so a row of them reads as one row rather than as a
 * big button next to two small ones.
 *
 *   primary    acid fill, `onAcid` label -- the one CTA per screen
 *   secondary  surface fill, hairline outline, body ink
 *   tertiary   no chrome at all: a label and a chevron
 */

export type ActionTier = 'primary' | 'secondary' | 'tertiary';

const ANTD_TYPE: Record<ActionTier, 'primary' | 'default' | 'text'> = {
  primary: 'primary',
  secondary: 'default',
  tertiary: 'text',
};

export function ActionButton({
  tier = 'secondary',
  children,
  onClick,
  disabled,
  loading,
  icon,
  title,
  chevron,
}: {
  tier?: ActionTier;
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  /** Busy: the button stops responding and shows a spinner where the icon was. */
  loading?: boolean;
  /** Leading icon, sized by the caller -- this is not an icon-button. */
  icon?: ReactNode;
  title?: string;
  /** Trailing chevron, for the tertiary "View diff ›" shape. */
  chevron?: boolean;
}) {
  return (
    <Button
      type={ANTD_TYPE[tier]}
      title={title}
      onClick={onClick}
      disabled={disabled || loading}
      loading={loading}
      icon={icon}
    >
      {children}
      {chevron && ' ›'}
    </Button>
  );
}
