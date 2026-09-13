import { Check, Copy } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { IconButton } from './Button';

/** How long "Copied" stays up before the control asks the question again. */
const RESET_MS = 1600;

/**
 * Copy a string to the clipboard, with the confirmation on the button itself.
 *
 * The feedback is local state rather than a toast for one reason: the thing the
 * user needs to know is *which* control they pressed. A toast at the corner of the
 * window says something happened somewhere; a check mark where their cursor
 * already is answers the question without moving their eyes.
 *
 * It is a `Button`, so it is found as one -- `data-ui="button"`, an accessible
 * name, and the copy state on `[data-copy-state]` for the stylesheet and the
 * tests. Primitives do not take a `className`, and overriding that here would
 * mean a second button component to carry it.
 *
 * A rejection is a real outcome, not a bug: the webview can refuse the write when
 * the window has lost focus. The button then says so, because a copy that silently
 * did nothing is how a user pastes a stale path into a build command.
 */
export function CopyButton({
  value,
  label = 'Copy',
  size = 'sm',
}: {
  /** What goes on the clipboard, verbatim. */
  value: string;
  /** The accessible name before the first press. */
  label?: string;
  size?: 'sm' | 'md' | 'lg';
}) {
  const [state, setState] = useState<'idle' | 'done' | 'failed'>('idle');
  const timer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const copy = async () => {
    let ok = true;
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      ok = false;
    }
    setState(ok ? 'done' : 'failed');
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setState('idle'), RESET_MS);
  };

  return (
    <IconButton
      icon={state === 'done' ? Check : Copy}
      tier="ghost"
      size={size}
      aria-live={state === 'failed' ? 'assertive' : 'polite'}
      data-copy-state={state}
      label={state === 'done' ? 'Copied' : state === 'failed' ? 'Copy failed' : label}
      onClick={copy}
    />
  );
}
