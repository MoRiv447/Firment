import { useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent } from 'react';

import styles from './Splitter.module.css';

/** How far one key press moves the edge, in px. */
const STEP = 16;

const clamp = (next: number, min: number, max: number) => Math.min(max, Math.max(min, next));

/**
 * The drag handle on the inspector's left edge.
 *
 * It exists because the column's honest width depends on what is in it: 360px is
 * right for a diff and wrong for a laptop, and the thing that loses text when
 * nothing shrinks is the transcript. A clamp is a guess about someone else's
 * window; this is a control.
 *
 * Directions are written from the column's point of view, not the pointer's:
 * dragging **left** widens the inspector, because the edge being moved is its left
 * one. `role="separator"` with `aria-valuenow` in pixels is what a screen reader
 * needs to report the same thing, and the arrow keys move the same edge, so the
 * keyboard and the mouse agree about which way is bigger.
 *
 * The width itself lives on the grid container as `--inspector-w`, which the caller
 * owns: a handle that set `style.width` would take the column's sizing out of the
 * cascade, where the media queries live.
 */
export function Splitter({
  value,
  min,
  max,
  onResize,
  label = 'Inspector width',
}: {
  /** The column's current width, in px. */
  value: number;
  min: number;
  max: number;
  onResize: (next: number) => void;
  label?: string;
}) {
  const origin = useRef({ x: 0, width: 0 });
  const [resizing, setResizing] = useState(false);

  const commit = (next: number) => onResize(clamp(next, min, max));

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    origin.current = { x: event.clientX, width: value };
    // Capture, not a window listener: the element keeps the pointer even when the
    // drag leaves the window, and the release cannot be missed.
    event.currentTarget.setPointerCapture(event.pointerId);
    setResizing(true);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!resizing) return;
    commit(origin.current.width - (event.clientX - origin.current.x));
  };

  const stop = (event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setResizing(false);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') commit(value + STEP);
    else if (event.key === 'ArrowRight') commit(value - STEP);
    else return;
    // Only once the key has been used: arrows belong to the rest of the shell
    // until this control has focus and asks for one.
    event.preventDefault();
  };

  return (
    <>
      <div
        data-ui="splitter"
        data-resizing={resizing || undefined}
        role="separator"
        aria-orientation="vertical"
        aria-label={label}
        aria-valuenow={Math.round(value)}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuetext={`${Math.round(value)} pixels`}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={stop}
        onPointerCancel={stop}
        onKeyDown={onKeyDown}
        className={styles.handle}
      />
      {resizing && <div aria-hidden className={styles.mask} />}
    </>
  );
}
