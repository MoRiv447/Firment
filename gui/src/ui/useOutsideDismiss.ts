import { useEffect, useRef } from 'react';
import type { RefObject } from 'react';

/**
 * Close a floating panel when the pointer goes somewhere else.
 *
 * `pointerdown`, not `click`: the app has rows that reorder on mouseup, buttons
 * that unmount on the first press, and text the user drags to select -- all three
 * end without a `click` on the original target, and a panel that only listens for
 * `click` stays open over the content the user is trying to reach. It is also the
 * event the rest of the tree (the workbench's hand-written `mousedown` in
 * `ChatView.tsx` used to) has to be reasoned about.
 *
 * A press inside *any* of `refs` is ignored, and that single rule covers two
 * different accidents. Presses inside the panel are inside because the panel is
 * portalled next to `#root` rather than behind it, so "is the target my own
 * subtree" cannot be asked of the DOM parent chain. And a press on the trigger is
 * inside because closing on `pointerdown` and then letting the `click` that
 * follows run the trigger's toggle would re-open the panel in the same gesture --
 * the flicker that makes a dropdown feel broken on a slow machine.
 */
export function useOutsideDismiss(options: {
  active: boolean;
  refs: Array<RefObject<HTMLElement | null> | null | undefined>;
  onDismiss: () => void;
}): void {
  const { active, refs } = options;

  // Same reason as in `useFocusTrap`: `onDismiss` is an inline arrow at every
  // call site, and resubscribing the listener on each render would be both
  // wasteful and a source of missed events during a render burst.
  const dismiss = useRef(options.onDismiss);
  dismiss.current = options.onDismiss;

  useEffect(() => {
    if (!active) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      for (const ref of refs) {
        if (ref?.current?.contains(target)) return;
      }
      dismiss.current();
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
    // `refs` is a fresh array literal at every call site; its identity says
    // nothing about whether the nodes it points at changed, so the effect keys on
    // `active` alone and reads whatever the array holds when the press happens.
  }, [active]);
}
