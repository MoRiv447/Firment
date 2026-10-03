import { useEffect, useRef } from 'react';

/**
 * `Ctrl`/`Cmd` + `N` opens a session, the same action the rail's button performs.
 *
 * It lives here rather than inline in `App` for one reason: as a hook it can be
 * mounted alone and asked "what happens when this key arrives", which is the only
 * way to test a window-level listener while no App-level render test exists.
 *
 * The handler is held in a ref and refreshed each render, so the subscription is
 * made once. Without that, the effect would re-subscribe on every render (the
 * handler closes over fresh state each time) and a key pressed mid-render would be
 * heard twice or not at all.
 */
export function useNewSessionShortcut(onNew: () => void, blocked = false): void {
  const handler = useRef(onNew);
  handler.current = onNew;
  const blockedRef = useRef(blocked);
  blockedRef.current = blocked;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      if (event.key.toLowerCase() !== 'n') return;
      // While a dialog is up the keystroke belongs to the dialog. Starting a
      // session underneath an unanswered question or an unapproved tool call would
      // strand it, and the rail's own button is disabled in that state anyway.
      if (blockedRef.current) return;
      event.preventDefault();
      handler.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
