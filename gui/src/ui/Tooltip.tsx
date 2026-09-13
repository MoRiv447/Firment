import { useEffect, useId, useRef, useState } from 'react';
import type { DOMAttributes, ReactNode, RefObject } from 'react';

import { Popover } from './Popover';
import styles from './Tooltip.module.css';

/** How long the pointer has to hold still before the tooltip appears, in ms. */
export const TOOLTIP_DELAY = 400;

/**
 * What `useTooltip` hands back: the state, the anchor, and the attributes that
 * belong on the element being hovered.
 *
 * The trigger is deliberately not wrapped. A `<span>` around a tooltip's target
 * sounds free and is not: the sidebar row is a grid item, the composer button is a
 * flex child, and an anonymous inline box between them and their container is a
 * layout bug that only shows up in the one row that was given a tooltip.
 */
export interface TooltipController<T extends HTMLElement = HTMLElement> {
  /** Put this on the element the tooltip is about. */
  anchorRef: RefObject<T | null>;
  /** Spread on the same element. Carries the pointer, focus and Escape handling. */
  triggerProps: Pick<
    DOMAttributes<HTMLElement>,
    'onPointerEnter' | 'onPointerLeave' | 'onFocus' | 'onBlur' | 'onKeyDown'
  > & { 'aria-describedby'?: string };
  /** For a parent that wants to close a child's tooltip, e.g. a row being clicked. */
  close: () => void;
  id: string;
  open: boolean;
}

/**
 * Arm, fire and dismiss a tooltip, without owning any of its markup.
 *
 * Call this in the component that renders the trigger, then pass the result to
 * `<Tooltip>`. Two pieces because the app's tooltips sit on controls this layer
 * does not own.
 *
 * The type parameter is the anchor's element, and it is there for the ref: a
 * `<button>` will not accept a `RefObject<HTMLElement | null>`, so a caller that
 * writes `useTooltip<HTMLButtonElement>()` can put `anchorRef` straight on the
 * control. `Tooltip` accepts any of them, since a narrower anchor is a valid
 * `HTMLElement`.
 */
export function useTooltip<T extends HTMLElement = HTMLElement>(
  delay: number = TOOLTIP_DELAY,
): TooltipController<T> {
  const id = useId();
  const anchorRef = useRef<T | null>(null);
  const timer = useRef<number | null>(null);
  const [open, setOpen] = useState(false);
  /** Set by Escape. The pointer leaving is what clears it, not time passing. */
  const dismissed = useRef(false);

  const cancel = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  };

  useEffect(() => cancel, []);

  const release = () => {
    dismissed.current = false;
    cancel();
    setOpen(false);
  };

  return {
    id,
    open,
    anchorRef,
    close: release,
    triggerProps: {
      'aria-describedby': open ? id : undefined,
      onPointerEnter: (event) => {
        // A tap fires pointerenter before pointerdown, and a tooltip that survives
        // the tap becomes an obstacle sitting on the row the user just chose.
        if (event.pointerType === 'touch' || dismissed.current || timer.current !== null) return;
        timer.current = window.setTimeout(() => {
          timer.current = null;
          setOpen(true);
        }, delay);
      },
      onPointerLeave: release,
      onFocus: () => {
        // No delay on focus: a keyboard user has already made an exact, deliberate
        // choice about which control to look at.
        if (dismissed.current) return;
        cancel();
        setOpen(true);
      },
      onBlur: release,
      onKeyDown: (event) => {
        if (event.key !== 'Escape') return;
        // Not prevented -- Escape belongs to whatever this tooltip is inside of.
        // This only takes the tooltip out of the way until the pointer leaves.
        dismissed.current = true;
        cancel();
        setOpen(false);
      },
    },
  };
}

/**
 * The tooltip's panel.
 *
 * The delay lives in `useTooltip`; the position and the shared chrome live in
 * `Popover`. What lives here is the one rule that is specific to a tooltip: it is
 * `pointer-events: none`, because a panel that appears under the cursor is a panel
 * that steals the hover from the control it describes, which reads as a control
 * that cannot be clicked.
 *
 * A tooltip must never be the only place an answer exists. The old tree kept a
 * session's full path in a tooltip alone -- invisible to a keyboard user, to a
 * touch screen and to anyone whose tooltip was closed.
 */
export function Tooltip({
  tip,
  text,
  side = 'top',
}: {
  tip: TooltipController;
  text: ReactNode;
  side?: 'top' | 'bottom' | 'left' | 'right';
}) {
  return (
    <Popover
      open={tip.open}
      anchorRef={tip.anchorRef}
      onClose={tip.close}
      role="tooltip"
      id={tip.id}
      side={side}
      align="center"
      className={styles.tip}
    >
      {text}
    </Popover>
  );
}
