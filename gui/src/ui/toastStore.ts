import { useSyncExternalStore } from 'react';

/**
 * The toast queue, outside React.
 *
 * Why a module store rather than a context: the callers are error paths in
 * `api.ts` wrappers, `await` chains in the workbench and a Tauri event handler --
 * none of them components, and all of them already import plain functions from this
 * layer. A context would mean threading a `push` function down every tree that
 * could fail, and the failures that matter most are the ones in code that has no
 * provider in scope.
 *
 * What the queue guarantees is the part worth reading:
 *
 * * **Four on screen, newest last.** A retry loop that fails twelve times must not
 *   bury the window in twelve panels; the oldest is dropped to make room.
 * * **Six seconds, then gone.** Nothing waits for a click, and no toast is
 *   preserved across a navigation -- if it mattered, it belongs in the transcript,
 *   where it can be read again.
 */

export type ToastTone = 'info' | 'ok' | 'failed' | 'attention';

export interface ToastItem {
  id: number;
  text: string;
  tone: ToastTone;
}

/** How long a toast stays up. Long enough to read a sentence, not a paragraph. */
export const TOAST_LIFETIME_MS = 6000;

/** The most the stack will ever show. */
export const TOAST_MAX = 4;

let items: ToastItem[] = [];
let nextId = 1;
const listeners = new Set<() => void>();

function publish(): void {
  for (const listener of listeners) listener();
}

/** Add a toast and start its timer. Returns the id, so a caller can dismiss it. */
export function pushToast(text: string, tone: ToastTone = 'info'): number {
  const id = nextId;
  nextId += 1;
  items = [...items, { id, text, tone }].slice(-TOAST_MAX);
  publish();
  window.setTimeout(() => dismissToast(id), TOAST_LIFETIME_MS);
  return id;
}

export function dismissToast(id: number): void {
  const before = items.length;
  items = items.filter((item) => item.id !== id);
  if (items.length !== before) publish();
}

/** Clear the stack, e.g. when the user leaves the view the toasts belong to. */
export function clearToasts(): void {
  if (items.length === 0) return;
  items = [];
  publish();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The current stack. A new array only when it changed, which is what lets React cache it. */
export function getToasts(): ToastItem[] {
  return items;
}

/** The viewport's view of the queue. */
export function useToasts(): ToastItem[] {
  return useSyncExternalStore(subscribe, getToasts, getToasts);
}
