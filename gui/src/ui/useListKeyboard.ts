import { useRef } from 'react';
import type { KeyboardEvent } from 'react';

/** How long a typeahead buffer stays open, in ms. Long enough for two keys. */
const TYPEAHEAD_MS = 600;

/**
 * Roster movement for an `aria-activedescendant` list: arrows, Home/End, wrap,
 * typeahead, Enter/Tab to commit, Escape to close.
 *
 * The list of "keyboard support" behaviours a dropdown needs is settled, and
 * getting any one of them wrong is what makes a hand-rolled control feel
 * cheaper than the library it replaced. Two of them are not obvious:
 *
 * * **Escape calls `preventDefault()` *and* `stopPropagation()`.** A Select that
 *   lives inside the Settings drawer is a React child of that drawer, so an
 *   Escape that only closed the list would keep bubbling and close the drawer
 *   with it. One key, one panel.
 * * **Tab commits rather than cancelling.** A keyboard user who has arrowed to
 *   the option they want and presses Tab means "that one, now move on"; treating
 *   Tab as a dismissal is how you end up saving the wrong model.
 */
export function useListKeyboard(options: {
  /** How many rows there are. `0` disables everything but Escape. */
  count: number;
  /** The highlighted row, or `-1` when nothing is. */
  active: number;
  onActive: (index: number) => void;
  onCommit?: (index: number) => void;
  onClose: () => void;
  /** Row labels, in order. Pass them to turn typeahead on. */
  labels?: string[];
  /** Wrap at the ends. On by default: a list that stops at the bottom wastes a keypress. */
  loop?: boolean;
  /**
   * Can this row be landed on? Defaults to "every row can".
   *
   * It exists because the two lists that use this index their rows differently.
   * `Menu` hands the hook a cursor over *selectable* rows only, so a separator or a
   * dead item is not in the arithmetic at all; `Select` keeps the whole option
   * list, because its `aria-activedescendant` has to name the row that is on
   * screen. Without this, the same keypress would skip dead rows in one control and
   * park on them in the other -- and a cursor on a row that refuses Enter reads as
   * a stuck list.
   */
  isEnabled?: (index: number) => boolean;
}): (event: KeyboardEvent) => void {
  const {
    count,
    active,
    onActive,
    onCommit,
    onClose,
    labels,
    loop = true,
    isEnabled = () => true,
  } = options;
  const typeahead = useRef({ buffer: '', at: 0 });

  /** The nearest row from `from` in direction `delta` that the cursor may rest on. */
  const reachable = (from: number, delta: number): number | undefined => {
    for (let offset = 1; offset <= count; offset += 1) {
      const raw = from + delta * offset;
      if (!loop && (raw < 0 || raw >= count)) return undefined;
      const index = ((raw % count) + count) % count;
      if (isEnabled(index)) return index;
    }
    return undefined;
  };

  const step = (delta: number) => {
    if (count === 0) return;
    const next = reachable(active, delta);
    if (next !== undefined) onActive(next);
  };

  const jump = (index: number) => {
    if (count === 0) return;
    // `index` itself first: `Home` on a list whose top row is enabled belongs on
    // the top row. Then outwards, so a dead end of the list still lands nearby.
    for (let offset = 0; offset < count; offset += 1) {
      const forward = (((index + offset) % count) + count) % count;
      if (isEnabled(forward)) {
        onActive(forward);
        return;
      }
      const backward = (((index - offset) % count) + count) % count;
      if (isEnabled(backward)) {
        onActive(backward);
        return;
      }
    }
  };

  /** Match a typed prefix, starting from the highlight so repeated keys cycle. */
  const matchPrefix = (typed: string) => {
    if (!labels || labels.length === 0) return;
    const needle = typed.toLowerCase();
    for (let offset = 1; offset <= labels.length; offset += 1) {
      const index = (active + offset) % labels.length;
      if (labels[index]?.toLowerCase().startsWith(needle)) {
        jump(index);
        return;
      }
    }
  };

  const onPrintable = (key: string) => {
    const now = Date.now();
    const state = typeahead.current;
    state.buffer = now - state.at > TYPEAHEAD_MS ? key : state.buffer + key;
    state.at = now;
    matchPrefix(state.buffer);
  };

  return (event: KeyboardEvent) => {
    const { key } = event;

    if (key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (key === 'ArrowDown') {
      event.preventDefault();
      step(1);
      return;
    }
    if (key === 'ArrowUp') {
      event.preventDefault();
      step(-1);
      return;
    }
    if (key === 'Home') {
      event.preventDefault();
      jump(0);
      return;
    }
    if (key === 'End') {
      event.preventDefault();
      jump(count - 1);
      return;
    }
    if (key === 'Enter' || key === ' ') {
      if (count === 0) return;
      // A space on a `<button>` trigger would otherwise activate it and toggle
      // the panel shut the instant it opened.
      event.preventDefault();
      onCommit?.(active < 0 ? 0 : active);
      return;
    }
    if (key === 'Tab') {
      if (active >= 0) onCommit?.(active);
      // Deliberately not prevented: focus has somewhere to go next, and
      // swallowing Tab would trap the user in the panel.
      onClose();
      return;
    }
    if (key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      onPrintable(key);
    }
  };
}
