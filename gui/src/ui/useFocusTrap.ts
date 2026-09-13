import { useEffect, useRef } from 'react';
import type { RefObject } from 'react';

import { acquireIsolation } from './portal';

/**
 * What counts as reachable by Tab inside a panel.
 *
 * `[inert]` is excluded because the browser will not focus it and neither will
 * we; `[tabindex="-1"]` is excluded because it is focusable by script only, and
 * a trap that counts it would offer a stop that Tab cannot actually land on.
 */
const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[contenteditable="true"]',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

/** The focusable elements of a subtree, in DOM order. */
export function getFocusable(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}

/**
 * Keep the keyboard inside `container` while `active`, and put it back after.
 *
 * Three separate jobs, all of which antd used to do and all of which are easy to
 * get half right:
 *
 * 1. On open, focus `[data-autofocus]`, else the first focusable, else the
 *    container itself (a dialog with no controls still has to take focus, or the
 *    user is reading a panel the screen reader has not announced).
 * 2. Tab and Shift+Tab wrap at the ends. `getFocusable()` is recomputed on each
 *    press, not cached -- a collapsed section inside a drawer changes the set
 *    while the dialog stays open.
 * 3. On close, focus returns to whatever had it. Without this, closing a
 *    permission dialog with Escape leaves the caret on a detached node and the
 *    next Tab starts from the top of the document.
 *
 * The rest of the app is made `inert` for the duration (see `portal.ts`), so the
 * wrap logic is a convenience rather than the only thing keeping a keyboard user
 * out of the background.
 */
export function useFocusTrap(
  container: RefObject<HTMLElement | null>,
  options: { active: boolean; onEscape?: () => void },
): void {
  const { active } = options;
  const previouslyFocused = useRef<HTMLElement | null>(null);

  // Held in a ref rather than a dependency on purpose: callers write
  // `onEscape={() => setOpen(false)}`, and a fresh closure every render would
  // re-run this effect -- which re-focuses the panel's first control and pulls
  // the caret out of the input the user is typing in.
  const escape = useRef(options.onEscape);
  escape.current = options.onEscape;

  useEffect(() => {
    if (!active) return;
    const node = container.current;
    if (!node) return;

    const activeEl = document.activeElement;
    previouslyFocused.current = activeEl instanceof HTMLElement ? activeEl : null;
    const releaseIsolation = acquireIsolation();

    const focusables = getFocusable(node);
    const target =
      node.querySelector<HTMLElement>('[data-autofocus]') ?? focusables[0] ?? node;
    if (target === node && node.tabIndex < 0) node.tabIndex = -1;
    target.focus({ preventScroll: true });

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        // Stop here so an Escape that closes a dialog does not also reach the
        // view behind it -- the same reason Select preventDefaults it.
        event.stopPropagation();
        escape.current?.();
        return;
      }
      if (event.key !== 'Tab') return;
      const items = getFocusable(node);
      if (items.length === 0) {
        event.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const current = document.activeElement;
      if (event.shiftKey && (current === first || current === node)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && current === last) {
        event.preventDefault();
        first.focus();
      }
    };

    node.addEventListener('keydown', onKeyDown);
    return () => {
      node.removeEventListener('keydown', onKeyDown);
      releaseIsolation();
      previouslyFocused.current?.focus({ preventScroll: true });
    };
  }, [active, container]);
}
