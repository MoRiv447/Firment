import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { useNewSessionShortcut } from '../shortcuts';

/**
 * The one window-level listener in the app, tested on its own.
 *
 * `App` has no render test (it needs the Tauri backend mocked), so a listener
 * written inline there would be untestable -- which is exactly how a shortcut that
 * fires twice, or swallows a keystroke a dialog needed, survives to a release.
 */

function press(init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key: 'n', cancelable: true, ...init });
  window.dispatchEvent(event);
  return event;
}

describe('the new-session shortcut', () => {
  it('fires on Ctrl+N and on Cmd+N, and takes the key away from the browser', () => {
    const onNew = vi.fn();
    renderHook(() => useNewSessionShortcut(onNew));
    expect(press({ ctrlKey: true }).defaultPrevented).toBe(true);
    press({ metaKey: true });
    expect(onNew).toHaveBeenCalledTimes(2);
  });

  it('ignores a bare n, and a combination that means something else', () => {
    const onNew = vi.fn();
    renderHook(() => useNewSessionShortcut(onNew));
    press({});
    press({ ctrlKey: true, altKey: true });
    press({ ctrlKey: true, key: 'b' });
    expect(onNew).not.toHaveBeenCalled();
  });

  it('stays quiet while a dialog owns the keyboard, and returns when it closes', () => {
    const onNew = vi.fn();
    const { rerender } = renderHook(({ blocked }) => useNewSessionShortcut(onNew, blocked), {
      initialProps: { blocked: true },
    });
    press({ ctrlKey: true });
    expect(onNew).not.toHaveBeenCalled();
    rerender({ blocked: false });
    press({ ctrlKey: true });
    expect(onNew).toHaveBeenCalledTimes(1);
  });

  it('stops listening once it is unmounted', () => {
    const onNew = vi.fn();
    const { unmount } = renderHook(() => useNewSessionShortcut(onNew));
    unmount();
    press({ ctrlKey: true });
    expect(onNew).not.toHaveBeenCalled();
  });
});
