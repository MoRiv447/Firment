import { describe, expect, it } from 'vitest';

import { isComposing } from '../ime';
import type { KeyboardEvent } from 'react';

/**
 * The one guard every text field is asked to use, and the two ways an input method reports itself.
 *
 * `isComposing` is the standard signal; `keyCode === 229` is what WebKit and Blink set for a
 * keystroke the IME consumed, including on engines that do not raise the flag for the Enter that
 * ends a composition. A field reading only one of the two works in one browser and sends half a
 * sentence in another, which is the split this function exists to keep in one place.
 */

/** Only the two signals the function reads are built here. A real synthetic event is far larger,
 * and pretending to be one would let this test drift from what the fields actually pass. */
const keystroke = (init: { isComposing?: boolean; keyCode?: number }): KeyboardEvent =>
  ({
    keyCode: init.keyCode ?? 0,
    // The standard flag lives on the native event, which is where React hands it through.
    nativeEvent: { isComposing: init.isComposing ?? false },
  }) as unknown as KeyboardEvent;

describe('isComposing', () => {
  it('reads the standard flag off the native event', () => {
    expect(isComposing(keystroke({ isComposing: true }))).toBe(true);
    expect(isComposing(keystroke({ isComposing: false }))).toBe(false);
  });

  it('reads the keyCode an engine that raises no flag still sets', () => {
    expect(isComposing(keystroke({ keyCode: 229 }))).toBe(true);
  });

  it('leaves an ordinary keystroke alone', () => {
    // The direction that would silently disable the control: a plain Enter (keyCode 13) with no
    // composition in progress must still reach the handler as "not composing".
    expect(isComposing(keystroke({ keyCode: 13 }))).toBe(false);
    expect(isComposing(keystroke({}))).toBe(false);
  });
});
