import { useRef } from 'react';
import { createPortal } from 'react-dom';
import type { ReactNode, RefObject } from 'react';

import { cx } from './cx';
import panel from './floating.module.css';
import { overlayRoot } from './portal';
import type { Align, Side } from './types';
import { usePopover } from './usePopover';

export interface PopoverProps {
  open: boolean;
  /** The control the panel hangs off. Its box is re-read on every resize. */
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  children: ReactNode;
  side?: Side;
  align?: Align;
  /** Distance from the anchor; `POPOVER_GAP` is what every panel in the app uses. */
  gap?: number;
  /** Close rather than chase a moving anchor. True except for a panel the user is scrolling. */
  closeOnScroll?: boolean;
  /** Never be narrower than the anchor. A `Select` listbox, and little else. */
  matchAnchorWidth?: boolean;
  /** Roomier: a panel holding prose or a form rather than a flush list of rows. */
  padded?: boolean;
  role?: 'menu' | 'listbox' | 'dialog' | 'tooltip' | 'group';
  /** A listbox whose rows toggle instead of replacing the selection. */
  multiselectable?: boolean;
  id?: string;
  labelledBy?: string;
  /** `Menu` moves focus into the panel, so it needs the node. */
  panelRef?: RefObject<HTMLDivElement | null>;
  /**
   * Primitives do not take a `className`, and this is the exception: the panel is
   * machinery, and one fact about it cannot be said from the outside any other
   * way -- a tooltip must not intercept pointer events.
   */
  className?: string;
}

/**
 * The floating panel: positioned, portalled, dismissed by a press elsewhere.
 *
 * `PopConfirm`, `Tooltip` and any panel that is just content are built on this, so
 * there is one answer to where a panel goes, one answer to what it is painted like
 * and one place that owns the `pointerdown` listener. `Menu` and `Select` use
 * `usePopover` directly instead, because for them the panel element *is* the
 * `role="menu"` / `role="listbox"` node and they have to own its ARIA attributes
 * and its keydown handler rather than forward them through here.
 *
 * It renders **nothing** while closed. Not a hidden `<div>`: the transcript has
 * dozens of hoverable rows, and a mounted-and-hidden panel per row is a cost paid
 * for a panel that is never open.
 *
 * The panel goes into `overlayRoot()` rather than staying where its trigger is,
 * because a `position: fixed` descendant of an ancestor with a `transform` is not
 * fixed, and the transcript pane is exactly the kind of ancestor that has one. See
 * `usePopover` for the full statement of that invariant.
 */
export function Popover({
  open,
  anchorRef,
  onClose,
  children,
  side = 'bottom',
  align = 'start',
  gap,
  closeOnScroll = true,
  matchAnchorWidth = false,
  padded = false,
  role,
  /** Only meaningful with `role="listbox"`: the panel answers Enter per row. */
  multiselectable = false,
  id,
  labelledBy,
  panelRef,
  className,
}: PopoverProps) {
  const ownRef = useRef<HTMLDivElement | null>(null);
  const ref = panelRef ?? ownRef;

  const { style, side: placed, ready } = usePopover({
    open,
    anchorRef,
    panelRef: ref,
    side,
    align,
    gap,
    closeOnScroll,
    onDismiss: onClose,
  });

  if (!open) return null;

  return createPortal(
    <div
      ref={ref}
      id={id}
      data-ui="popover"
      data-side={placed}
      data-ready={ready || undefined}
      data-padded={padded || undefined}
      role={role}
      aria-multiselectable={multiselectable || undefined}
      aria-labelledby={labelledBy}
      style={style}
      className={cx(panel.panel, matchAnchorWidth && panel.matchWidth, className)}
    >
      {children}
    </div>,
    overlayRoot(),
  );
}
