import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TOOLTIP_DELAY, Tooltip, ToastViewport, useTooltip } from '../';
import {
  TOAST_LIFETIME_MS,
  TOAST_MAX,
  clearToasts,
  dismissToast,
  getToasts,
  pushToast,
} from '../toastStore';

/**
 * The two controls that answer on a timer.
 *
 * A tooltip and a toast are the parts of the interface that fire without being
 * asked, which is also how they become noise: a tooltip that pops the instant the
 * mouse crosses a row follows the cursor across the whole sidebar, and a toast
 * that never leaves turns a retry loop into a wall. Both tests here are about the
 * timing, not the markup.
 */

function Tipped({ delay }: { delay?: number }) {
  const tip = useTooltip<HTMLButtonElement>(delay);
  return (
    <>
      <button type="button" ref={tip.anchorRef} {...tip.triggerProps}>
        Run
      </button>
      <Tooltip tip={tip} text="Send the current file" />
    </>
  );
}

describe('Tooltip', () => {
  it('waits for the pointer to settle, then describes the control', () => {
    vi.useFakeTimers();
    render(<Tipped delay={120} />);
    const trigger = screen.getByRole('button', { name: 'Run' });

    fireEvent.pointerEnter(trigger, { pointerType: 'mouse' });
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(trigger).not.toHaveAttribute('aria-describedby');

    act(() => {
      vi.advanceTimersByTime(120);
    });
    expect(screen.getByRole('tooltip')).toHaveTextContent('Send the current file');
    expect(trigger).toHaveAttribute('aria-describedby');

    fireEvent.pointerLeave(trigger);
    expect(screen.queryByRole('tooltip')).toBeNull();
    vi.useRealTimers();
  });

  it('does not chase a touch', () => {
    vi.useFakeTimers();
    render(<Tipped delay={120} />);
    const trigger = screen.getByRole('button', { name: 'Run' });
    // A tap fires pointerenter before pointerdown. A panel that survives the tap
    // sits on top of the row the user just chose.
    fireEvent.pointerEnter(trigger, { pointerType: 'touch' });
    act(() => {
      vi.advanceTimersByTime(TOOLTIP_DELAY * 2);
    });
    expect(screen.queryByRole('tooltip')).toBeNull();
    vi.useRealTimers();
  });

  it('opens the moment the keyboard arrives', () => {
    render(<Tipped />);
    const trigger = screen.getByRole('button', { name: 'Run' });
    // No delay: tabbing to a control is already an exact, deliberate choice.
    fireEvent.focus(trigger);
    expect(screen.getByRole('tooltip')).toBeInTheDocument();
    fireEvent.blur(trigger);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('gets out of the way on Escape without swallowing the key', () => {
    const behind = vi.fn();
    render(
      <div onKeyDown={behind}>
        <Tipped />
      </div>,
    );
    const trigger = screen.getByRole('button', { name: 'Run' });
    fireEvent.focus(trigger);
    expect(screen.getByRole('tooltip')).toBeInTheDocument();

    // `fireEvent` returns false only when something prevented the default.
    expect(fireEvent.keyDown(trigger, { key: 'Escape' })).toBe(true);
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(behind).toHaveBeenCalledTimes(1);

    // Escape gets it out of the way; it does not mute the control. Leaving and
    // coming back is a new, deliberate choice, so the tip is armed again.
    fireEvent.blur(trigger);
    fireEvent.focus(trigger);
    expect(screen.getByRole('tooltip')).toBeInTheDocument();
  });
});

describe('toast store', () => {
  beforeEach(() => clearToasts());
  afterEach(() => vi.useRealTimers());

  it('keeps the newest four and drops the oldest', () => {
    for (let i = 0; i < TOAST_MAX + 2; i += 1) pushToast(`attempt ${i + 1}`, 'failed');
    const shown = getToasts();
    expect(shown).toHaveLength(TOAST_MAX);
    // A retry loop that fails six times is four panels, and the one you can
    // still read is the latest attempt rather than the first.
    expect(shown.map((toast) => toast.text)).toEqual([
      'attempt 3',
      'attempt 4',
      'attempt 5',
      'attempt 6',
    ]);
  });

  it('expires on its own', () => {
    vi.useFakeTimers();
    pushToast('Flashing…', 'info');
    expect(getToasts()).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(TOAST_LIFETIME_MS - 1);
    });
    expect(getToasts()).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(getToasts()).toHaveLength(0);
  });

  it('dismisses by id, and a stale id is not an error', () => {
    const id = pushToast('Saved', 'ok');
    pushToast('Kept', 'attention');
    expect(getToasts()).toHaveLength(2);
    dismissToast(id);
    expect(getToasts().map((toast) => toast.text)).toEqual(['Kept']);
    expect(() => dismissToast(id)).not.toThrow();
    expect(() => clearToasts()).not.toThrow();
  });
});

describe('ToastViewport', () => {
  beforeEach(() => clearToasts());
  afterEach(() => vi.useRealTimers());

  it('exists before there is anything to announce', () => {
    render(<ToastViewport />);
    const stack = document.querySelector('[data-ui="toast-stack"]');
    expect(stack).toHaveAttribute('aria-live', 'polite');
    expect(stack).toBeEmptyDOMElement();
  });

  it('renders the tone and lets the user dismiss it', () => {
    vi.useFakeTimers();
    render(<ToastViewport />);
    act(() => {
      pushToast('Serial port busy', 'failed');
    });
    const toast = screen.getByText('Serial port busy').closest('[data-ui="toast"]');
    expect(toast).toHaveAttribute('data-tone', 'failed');
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText('Serial port busy')).toBeNull();
  });
});
