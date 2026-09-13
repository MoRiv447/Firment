/**
 * The node every floating panel portals into.
 *
 * Why a second root at all: a panel inside `#root` is trapped by whatever
 * `overflow` and `z-index` context its trigger sits in -- the transcript scrolls,
 * the inspector pane clips, and a `position: fixed` descendant of an ancestor
 * with a `transform` stops being fixed at all. Moving the panel to a child of
 * `body` escapes all three, and it also gives the focus trap a sibling it can
 * mark `inert` instead of having to walk the tree with a MutationObserver.
 *
 * Created on demand rather than written into index.html: a missing `<div>` there
 * would take every overlay down with it and print nothing, while this function
 * cannot fail.
 */

export const OVERLAY_ROOT_ID = 'overlay-root';

/** The overlay root, appending it to `body` the first time one is needed. */
export function overlayRoot(): HTMLElement {
  const existing = document.getElementById(OVERLAY_ROOT_ID);
  if (existing) return existing;
  const node = document.createElement('div');
  node.id = OVERLAY_ROOT_ID;
  document.body.appendChild(node);
  return node;
}

/**
 * Cut the app off from pointer and keyboard while a modal panel is open.
 *
 * `inert` rather than a `tabIndex` sweep or a focus listener on `#root`: the
 * browser removes the subtree from sequential focus navigation and from the
 * accessibility tree in one step, so a screen-reader user is not invited into a
 * window that is behind a scrim.
 *
 * Reference counted, because panels stack: a `PopConfirm` opened from inside a
 * Drawer releases its isolation on unmount, and if the flag belonged to whoever
 * wrote it last, that unmount would hand the keyboard back to an app that is
 * still behind a scrim.
 *
 * Returns the release function. Call it from an effect cleanup; a second call is
 * ignored, so a StrictMode double-invoke cannot drive the count negative.
 */
let openPanels = 0;

function applyIsolation(): void {
  const root = document.getElementById('root');
  if (root) root.inert = openPanels > 0;
}

export function acquireIsolation(): () => void {
  openPanels += 1;
  applyIsolation();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    openPanels -= 1;
    applyIsolation();
  };
}
