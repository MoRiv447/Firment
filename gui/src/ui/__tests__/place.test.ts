import { describe, expect, it } from 'vitest';

import { place, POPOVER_GAP } from '../usePopover';

/**
 * The placement arithmetic, tested on its own.
 *
 * jsdom has no layout engine -- every `getBoundingClientRect()` is zeroes and
 * `offsetWidth` is 0 -- so a rendered popover cannot be asked where it landed. This
 * is the reason `place()` is a pure function over plain numbers rather than a hook
 * that reads the DOM: the whole policy of "where does a panel go" is testable here,
 * and the hook is left with the parts that genuinely need a document.
 */
const anchor = { top: 100, left: 100, width: 200, height: 40 };
const panel = { width: 120, height: 80 };
const viewport = { width: 800, height: 600 };

describe('place()', () => {
  it('drops a panel below its anchor, aligned to its left edge', () => {
    const p = place(anchor, panel, viewport);
    expect(p).toMatchObject({
      side: 'bottom',
      top: anchor.top + anchor.height + POPOVER_GAP,
      left: anchor.left,
    });
  });

  it('hands back the anchor width, because the panel is not allowed to measure it again', () => {
    expect(place(anchor, panel, viewport).anchorWidth).toBe(anchor.width);
  });

  it('flips to the side with room', () => {
    const low = { ...anchor, top: 540 };
    const p = place(low, panel, viewport, 'bottom');
    expect(p.side).toBe('top');
    expect(p.top).toBe(low.top - POPOVER_GAP - panel.height);
  });

  it('keeps the wanted side when flipping would not help', () => {
    // A panel to the right of an anchor near the right edge: `left` would put it
    // over the anchor, not beside it, so the gap on both sides is what decides.
    const rightEdge = { top: 100, left: 740, width: 40, height: 20 };
    const p = place(rightEdge, panel, viewport, 'right');
    expect(p.side).toBe('left');
    expect(p.left).toBe(rightEdge.left - POPOVER_GAP - panel.width);
  });

  it('slides a panel back inside the window instead of hanging it off the edge', () => {
    const nearRight = { top: 100, left: 700, width: 120, height: 20 };
    const p = place(nearRight, panel, viewport, 'bottom', 'end');
    // `end` would put the panel's right edge on the anchor's, which is off-screen.
    expect(p.left + panel.width).toBeLessThanOrEqual(viewport.width - POPOVER_GAP);
    expect(p.left).toBe(672);
  });

  it('centres on the cross axis when asked', () => {
    expect(place(anchor, panel, viewport, 'bottom', 'center').left).toBe(
      anchor.left + (anchor.width - panel.width) / 2,
    );
  });

  it('reports the room left on the side it chose as the max height', () => {
    const p = place(anchor, panel, viewport);
    expect(p.maxHeight).toBe(viewport.height - (anchor.top + anchor.height) - POPOVER_GAP);
  });

  it('gives a panel that overflows the window a room it can scroll inside', () => {
    const short = { top: 500, left: 10, width: 40, height: 20 };
    const p = place(short, { width: 100, height: 400 }, viewport, 'top');
    expect(p.side).toBe('top');
    expect(p.maxHeight).toBe(short.top - POPOVER_GAP);
    expect(p.top).toBeGreaterThanOrEqual(POPOVER_GAP);
  });

  it('respects a custom gap', () => {
    expect(place(anchor, panel, viewport, 'bottom', 'start', 2).top).toBe(142);
  });
});
