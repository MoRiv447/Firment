import { useCallback, useLayoutEffect, useState } from 'react';
import type { CSSProperties, RefObject } from 'react';

import type { Align, Side } from './types';
import { useOutsideDismiss } from './useOutsideDismiss';

/** The distance a panel keeps from its anchor *and* from the window edge. */
export const POPOVER_GAP = 8;

/** A box in viewport coordinates. `DOMRect` satisfies it structurally. */
export interface Box {
  top: number;
  left: number;
  width: number;
  height: number;
}

/** Where a panel should go, in viewport coordinates. */
export interface Placed {
  top: number;
  left: number;
  /** The side it ended up on, which is not always the one that was asked for. */
  side: Side;
  /** Room left on that side: the panel's `max-height`. */
  maxHeight: number;
  /**
   * The anchor's own width.
   *
   * It travels with the position because nothing else is allowed to measure the
   * anchor: a `Select` listbox must be at least as wide as its trigger, and a
   * second `getBoundingClientRect()` somewhere else would be a second source for
   * a number that changes every time the window does.
   */
  anchorWidth: number;
}

const isVertical = (side: Side) => side === 'top' || side === 'bottom';
const opposite = (side: Side): Side =>
  side === 'top' ? 'bottom' : side === 'bottom' ? 'top' : side === 'left' ? 'right' : 'left';

/**
 * Put a panel next to an anchor, inside the viewport.
 *
 * A pure function taking plain numbers, because jsdom has no layout engine: every
 * `getBoundingClientRect()` in a test returns zeroes, so placement is the part of
 * a popover that can only be tested by calling the arithmetic directly.
 *
 * The order of the steps is the whole behaviour:
 *
 * 1. Ask how much room the wanted side has.
 * 2. If that is not enough and the opposite side has more, flip. Flip on the
 *    cross axis only if flipping could ever help -- a `left` panel that does not
 *    fit is not made better by `right` if the anchor is centred -- so this is a
 *    flip-or-keep test, not a search.
 * 3. Line the panel up along the cross axis, then clamp it back inside the
 *    window. Clamping last means a panel anchored near a corner slides inward
 *    instead of being pushed off the opposite edge by the flip.
 */
export function place(
  anchor: Box,
  panel: { width: number; height: number },
  viewport: { width: number; height: number },
  want: Side = 'bottom',
  align: Align = 'start',
  gap: number = POPOVER_GAP,
): Placed {
  const anchorBottom = anchor.top + anchor.height;
  const anchorRight = anchor.left + anchor.width;
  const room: Record<Side, number> = {
    top: anchor.top - gap,
    bottom: viewport.height - anchorBottom - gap,
    left: anchor.left - gap,
    right: viewport.width - anchorRight - gap,
  };

  const need = isVertical(want) ? panel.height : panel.width;
  const other = opposite(want);
  const side = room[want] < need && room[other] > room[want] ? other : want;

  let top: number;
  let left: number;
  if (isVertical(side)) {
    top = side === 'top' ? anchor.top - gap - panel.height : anchorBottom + gap;
    left =
      align === 'end'
        ? anchorRight - panel.width
        : align === 'center'
          ? anchor.left + (anchor.width - panel.width) / 2
          : anchor.left;
  } else {
    left = side === 'left' ? anchor.left - gap - panel.width : anchorRight + gap;
    top =
      align === 'end'
        ? anchorBottom - panel.height
        : align === 'center'
          ? anchor.top + (anchor.height - panel.height) / 2
          : anchor.top;
  }

  const maxLeft = Math.max(gap, viewport.width - panel.width - gap);
  const maxTop = Math.max(gap, viewport.height - panel.height - gap);
  const clampedTop = Math.min(Math.max(top, gap), maxTop);

  const roomOnSide = isVertical(side)
    ? side === 'top'
      ? anchor.top - gap
      : viewport.height - anchorBottom - gap
    : viewport.height - clampedTop - gap;

  return {
    top: clampedTop,
    left: Math.min(Math.max(left, gap), maxLeft),
    side,
    maxHeight: Math.max(0, roomOnSide),
    anchorWidth: anchor.width,
  };
}

/**
 * Position, dismiss and scroll policy for a floating panel.
 *
 * The panel must be portalled into `overlayRoot()` (see `portal.ts`), and there is
 * an invariant that only shows up when it is broken: **no ancestor of the anchor
 * may carry `transform`, `filter` or `will-change`.** A fixed element is fixed
 * relative to the nearest transformed ancestor, so one `translateZ(0)` on a pane
 * would move a menu to the wrong place -- and only in the one pane that has it,
 * which is why this is written down. It is the reason the inspector resizes with
 * `width` rather than with a transform.
 *
 * No `autoUpdate` loop and no scroll-following: nothing is scrolling underneath an
 * open panel in this app except the transcript, and there the right answer is to
 * close rather than to chase a moving anchor.
 */
export function usePopover(options: {
  open: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  panelRef: RefObject<HTMLElement | null>;
  side?: Side;
  align?: Align;
  gap?: number;
  /** Close instead of repositioning when an ancestor scrolls. */
  closeOnScroll?: boolean;
  onDismiss?: () => void;
}): { style: CSSProperties; side: Side; ready: boolean; refresh: () => void } {
  const {
    open,
    anchorRef,
    panelRef,
    side: want = 'bottom',
    align = 'start',
    gap = POPOVER_GAP,
    closeOnScroll = true,
    onDismiss,
  } = options;

  const [placed, setPlaced] = useState<Placed | null>(null);

  const measure = useCallback(() => {
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    setPlaced(
      place(
        anchor.getBoundingClientRect(),
        { width: panel.offsetWidth, height: panel.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
        want,
        align,
        gap,
      ),
    );
  }, [anchorRef, panelRef, want, align, gap]);

  // Layout effect, not effect: a paint with the panel at its unpositioned corner
  // is a visible jump, and this is the one place in the app where "before paint"
  // is the difference between correct and broken.
  useLayoutEffect(() => {
    if (!open) {
      setPlaced(null);
      return;
    }
    measure();
  }, [open, measure]);

  const dismiss = useCallback(() => onDismiss?.(), [onDismiss]);
  const refresh = measure;

  useOutsideDismiss({ active: open, refs: [anchorRef, panelRef], onDismiss: dismiss });

  useLayoutEffect(() => {
    if (!open) return;
    const onResize = () => measure();
    const onScroll = () => {
      if (closeOnScroll) dismiss();
      else measure();
    };
    window.addEventListener('resize', onResize);
    // Capture: the panels that overflow live inside scrolling containers, and a
    // scroll event does not bubble.
    document.addEventListener('scroll', onScroll, { capture: true });
    return () => {
      window.removeEventListener('resize', onResize);
      document.removeEventListener('scroll', onScroll, { capture: true });
    };
  }, [open, closeOnScroll, dismiss, measure]);

  // Only custom properties cross into the DOM from here. The gate rule
  // (`no-literal-tokens.test.ts`, G3) allows an inline style to carry a token and
  // nothing else, which keeps the actual positioning rules in the stylesheet
  // where the rest of the visual system lives.
  const style = (
    placed
      ? {
          '--pop-top': `${placed.top}px`,
          '--pop-left': `${placed.left}px`,
          '--pop-max-h': `${placed.maxHeight}px`,
          '--pop-anchor-w': `${placed.anchorWidth}px`,
        }
      : {}
  ) as CSSProperties;

  return { style, side: placed?.side ?? want, ready: placed !== null, refresh };
}
