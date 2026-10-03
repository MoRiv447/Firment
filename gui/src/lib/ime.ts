import type { KeyboardEvent } from 'react';

/**
 * Whether this keystroke belongs to the input method, not to the field.
 *
 * A person typing Chinese, Japanese or Korean does not type a letter and get a letter: the keys
 * go into a composition buffer and the IME offers candidates. Confirming a candidate is the
 * **Enter** key, and moving between candidates is **ArrowDown/ArrowUp** -- and the DOM reports
 * both as ordinary keystrokes with `isComposing` set. Handled as a submit, that keystroke sends
 * the message the person is still writing; handled as a list command, it moves a cursor the IME
 * was already using. The field's `value` cannot be the guard either, because during composition
 * the candidate has not been written into it yet.
 *
 * `keyCode === 229` is the same fact from the older side: WebKit and Blink report the IME's own
 * keystrokes as keyCode 229, and Safari for years did not set `isComposing` on the Enter that
 * *ends* a composition. Checking both costs nothing and the second one only fires for an engine
 * that has no other way to say it.
 *
 * Every surface in this app is English, but every person using it is not: the workbench panes,
 * the composer, the ask dialog and the settings rows all take free text.
 */
export function isComposing(event: KeyboardEvent): boolean {
  return event.nativeEvent.isComposing || event.keyCode === 229;
}
