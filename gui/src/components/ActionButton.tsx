import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from 'antd';
import { LoadingOutlined } from '@ant-design/icons';
import { color, font, radius, slant, slantClip } from '../styles/tokens';
import type { SlantEdge } from '../styles/tokens';

/**
 * The three button weights.
 *
 * `secondary` and `tertiary` are antd `Button`s: not one pixel of chrome is
 * hand-drawn, so hover, active, disabled and loading come from the theme rather
 * than from this file.
 *
 * `primary` is drawn here, because it carries the slant and a `clip-path` cut
 * cannot be a `border`: the fill is an absolutely-positioned clipped layer
 * behind the label, and the edge is a second clipped layer with the fill inset
 * 1px inside it. A `border` on a clipped element exists only on the four box
 * edges, so the diagonal would come out with no edge at all.
 *
 * # The edge has its own token now
 *
 * The edge layer used to be filled with `color.outline`, which was pure black
 * while the whole interface was framed in black. When `outline` became an
 * ordinary border grey, that layer silently became a grey ring around the acid
 * fill -- a green button that looks outlined by mistake, which is exactly how it
 * read. It is `color.brandEdge` now: the fill does not change with the ground,
 * so its edge is not allowed to either, and no generic border token can reach
 * it.
 *
 * # Why the label is not skewed
 *
 * The cut is geometry, never a `transform: skewX`. A transform shears the type
 * with the shape, and Latin text leaning is a decal rather than a design.
 * `clip-path` cuts geometry and leaves type alone.
 *
 * # Scope
 *
 * One cut per screen, and only on `primary`. A slant on every button dilutes it
 * into decoration.
 */

export type ActionTier = 'primary' | 'secondary' | 'tertiary';

export function ActionButton({
  tier = 'secondary',
  edge,
  children,
  onClick,
  disabled,
  loading,
  icon,
  title,
  chevron,
}: {
  tier?: ActionTier;
  /** Override the primary tier's default cut. `both` makes a parallelogram. */
  edge?: SlantEdge;
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
  // Hover and focus are state rather than CSS because the GUI has no stylesheet:
  // every value is an inline token read (docs/design/tokens.md).
  const [hover, setHover] = useState(false);
  const [focused, setFocused] = useState(false);
  const inert = disabled || loading;

  if (tier !== 'primary') {
    return (
      <Button
        type={tier === 'tertiary' ? 'text' : 'default'}
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

  const shape: SlantEdge = edge ?? 'left';
  const clip = slantClip(shape);
  const clipped = clip !== undefined;

  return (
    <button
      type="button"
      title={title}
      onClick={inert ? undefined : onClick}
      disabled={inert}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      style={{
        position: 'relative',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: slant.controlHeight,
        padding: '0 16px',
        // A radius on the cut shape would round over the diagonal and eat it;
        // a clipped control stays square (docs/design/tokens.md).
        borderRadius: clipped ? 0 : radius.control,
        border: 'none',
        background: 'transparent',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
        // A rectangle rather than a cut outline, on purpose: the ring has to be
        // visible to a keyboard user, and tracing the diagonal is not worth an
        // unfollowable focus indicator.
        outline: focused ? `2px solid ${color.focusRing}` : 'none',
        outlineOffset: 2,
        // Reset the UA button styles; every value above is ours.
        font: 'inherit',
        textAlign: 'center',
      }}
    >
      {/* The edge, then the fill inset one pixel inside it. */}
      <span
        aria-hidden
        style={{ position: 'absolute', inset: 0, background: color.brandEdge, clipPath: clip }}
      />
      <span
        aria-hidden
        style={{
          position: 'absolute',
          inset: 1,
          background: color.brandAcid,
          clipPath: clip,
          // A translucent wash would let the edge layer show through the
          // diagonal, so hover is a brightness shift instead.
          filter: hover && !inert ? 'brightness(0.94)' : undefined,
        }}
      />
      <span
        style={{
          position: 'relative',
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          // Always `onAcid`. The grey this used to switch to while inert is
          // 2.02:1 on the dark fill and 3.80:1 on the light one -- unreadable,
          // and identical for "you cannot use this" and "it is working".
          // Disabled is carried by the opacity above; loading by the spinner.
          color: color.onAcid,
          fontWeight: 600,
          fontFamily: font.sans,
          // The cut steals from the left, so the label needs it back or it
          // reads as off-centre (see `slant.opticalPadLeft`).
          paddingLeft: clipped && shape === 'left' ? slant.opticalPadLeft : 0,
        }}
      >
        {loading ? <LoadingOutlined /> : icon}
        {children}
        {chevron && ' ›'}
      </span>
    </button>
  );
}
