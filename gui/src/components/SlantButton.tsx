import type { ReactNode } from 'react';
import { color, font, radius, slant } from '../styles/tokens';

/**
 * The three button weights, and the only place the slant is drawn.
 *
 * Weight is carried by **colour and fill, never by size**: all three are the
 * same height (`slant.controlHeight`), so a row of them reads as one row rather
 * than as a big button next to two small ones.
 *
 *   primary    acid fill, `onAcid` label -- the one CTA per screen
 *   secondary  surface fill, hairline outline, body ink
 *   tertiary   no chrome at all: a label and a chevron
 *
 * # Why the fill is a separate layer
 *
 * The slant is a `clip-path`, and clipping the *button* would clip its label,
 * its focus ring and its border along with the fill. So the fill is an
 * absolutely-positioned layer behind the label and the button itself stays
 * unclipped: nothing but the coloured shape is cut.
 *
 * This is also why the slant is never a `skewX` on the element. A transform
 * would shear the text with the shape -- the label would lean, and Latin text
 * leaning is a decal, not a design. `clip-path` cuts geometry and leaves type
 * alone (docs/design/tokens.md, "the slant").
 */

export type SlantButtonTier = 'primary' | 'secondary' | 'tertiary';

/** Which edges the cut is made on. `left` is the shipped default. */
export type SlantEdge = 'left' | 'both' | 'none';

/**
 * The cut, in CSS.
 *
 * `left`: the bottom-left triangle is removed, so the left edge runs from the
 * very top-left down and inward. Removed area sits on the left, which is what
 * `slant.opticalPadLeft` compensates for.
 */
export function slantClip(edge: SlantEdge): string | undefined {
  const cut = `${slant.cut}px`;
  switch (edge) {
    case 'left':
      return `polygon(0 0, 100% 0, 100% 100%, ${cut} 100%)`;
    case 'both':
      return `polygon(${cut} 0, 100% 0, calc(100% - ${cut}) 100%, 0 100%)`;
    default:
      return undefined;
  }
}

/** The cut defaults to the primary tier; the other two are square. */
const defaultEdge = (tier: SlantButtonTier): SlantEdge => (tier === 'primary' ? 'left' : 'none');

export function SlantButton({
  tier = 'secondary',
  edge,
  children,
  onClick,
  disabled,
  title,
  chevron,
}: {
  tier?: SlantButtonTier;
  /** Override the tier's default. `both` makes a parallelogram. */
  edge?: SlantEdge;
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
  /** Trailing chevron, for the tertiary "View diff ›" shape. */
  chevron?: boolean;
}) {
  const shape = edge ?? defaultEdge(tier);
  const clipped = slantClip(shape) !== undefined;

  const label = (
    <span
      style={{
        position: 'relative',
        color: disabled
          ? color.stepPendingInk
          : tier === 'primary'
            ? color.onAcid
            : tier === 'tertiary'
              ? color.muted
              : color.ink,
        fontWeight: tier === 'primary' ? 600 : 500,
        fontFamily: font.sans,
        // The cut steals from the left, so the label needs it back or it reads
        // as off-centre (see `slant.opticalPadLeft`).
        paddingLeft: clipped && shape === 'left' ? slant.opticalPadLeft : 0,
      }}
    >
      {children}
      {chevron && ' ›'}
    </span>
  );

  return (
    <button
      type="button"
      title={title}
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      style={{
        position: 'relative',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: slant.controlHeight,
        padding: '0 16px',
        // `radius.control` on the cut shape would round over the diagonal and
        // eat it; the clipped tiers stay square (docs/design/tokens.md).
        borderRadius: clipped ? 0 : radius.control,
        border: tier === 'secondary' ? `1px solid ${color.lineStrong}` : '1px solid transparent',
        background: 'transparent',
        cursor: disabled ? 'not-allowed' : 'pointer',
        // Reset the UA button styles; every value above is ours.
        font: 'inherit',
        textAlign: 'center',
        opacity: disabled ? 0.6 : 1,
      }}
    >
      {/*
        The dark edge, drawn as an outer layer with the fill inset one pixel
        inside it. A `border` on a clipped element only exists on the four box
        edges -- the clip then cuts the diagonal, and the diagonal comes out
        with no edge at all. Nesting two clipped layers gives the cut an edge
        the border property cannot.
      */}
      {tier === 'primary' && (
        <span
          aria-hidden
          style={{
            position: 'absolute',
            inset: 0,
            background: color.outline,
            clipPath: slantClip(shape),
          }}
        />
      )}
      <span
        aria-hidden
        style={{
          position: 'absolute',
          inset: tier === 'primary' ? 1 : 0,
          background:
            tier === 'primary'
              ? color.brandAcid
              : tier === 'secondary'
                ? color.surface
                : 'transparent',
          clipPath: slantClip(shape),
        }}
      />
      {label}
    </button>
  );
}
