import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { Button, Confirm, Drawer, Modal, PopConfirm, confirm } from '..';

/**
 * What a window does to the keyboard.
 *
 * A dialog is the one place in this app where the user is deliberately cut off
 * from everything else, and there are exactly four ways to get that wrong, all of
 * them invisible to a screenshot: focus that never goes in, Tab that leaks out the
 * end, focus that does not come back, and a background still reachable while
 * something is asking a question. Each test here is one of those.
 */

const app = () => document.getElementById('root') as HTMLElement;

const scrim = () => {
  const node = document.querySelector<HTMLElement>('[data-ui="scrim"]');
  if (!node) throw new Error('no scrim rendered');
  return node;
};

function Panel({ onClose, children }: { onClose?: () => void; children?: ReactNode }) {
  const ref = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" ref={ref} onClick={() => setOpen(true)}>
        Open
      </button>
      <Modal open={open} title="Serial port" onClose={onClose ?? (() => setOpen(false))}>
        {children ?? (
          <>
            <p>Choose a port.</p>
            <Button>First</Button>
            <Button>Second</Button>
          </>
        )}
      </Modal>
    </>
  );
}

/** Open the way a user does: the trigger has focus, then the panel appears. */
function openPanel(onClose?: () => void) {
  render(<Panel onClose={onClose} />);
  const trigger = screen.getByRole('button', { name: 'Open' });
  trigger.focus();
  fireEvent.click(trigger);
  return trigger;
}

describe('Modal', () => {
  it('moves focus into the panel and marks the app inert', () => {
    openPanel();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(app().inert).toBe(true);
    // Nothing asks for autofocus here, so the trap takes the first control: a
    // dialog the screen reader never entered is a dialog that never opened.
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();
  });

  it('wraps Tab at both ends instead of leaking into the inert app', () => {
    openPanel();
    const last = screen.getByRole('button', { name: 'Second' });
    const close = screen.getByRole('button', { name: 'Close' });

    last.focus();
    fireEvent.keyDown(last, { key: 'Tab' });
    expect(close).toHaveFocus();

    close.focus();
    fireEvent.keyDown(close, { key: 'Tab', shiftKey: true });
    expect(last).toHaveFocus();
  });

  it('gives focus back to whatever opened it', () => {
    const trigger = openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(trigger).toHaveFocus();
    expect(app().inert).toBe(false);
  });

  it('closes on Escape and stops the key there', () => {
    const onClose = vi.fn();
    const behind = vi.fn();
    render(
      <div onKeyDown={behind}>
        <Panel onClose={onClose} />
      </div>,
    );
    const trigger = screen.getByRole('button', { name: 'Open' });
    trigger.focus();
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(behind).not.toHaveBeenCalled();
  });

  it('treats a press past the scrim as a dismiss', () => {
    const onClose = vi.fn();
    openPanel(onClose);
    fireEvent.pointerDown(scrim());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('holds the line when the answer is required', () => {
    const onClose = vi.fn();
    render(
      <Modal open title="Allow serial.write?" dismissable={false} onClose={onClose}>
        <p>The agent wants to send bytes to COM3.</p>
      </Modal>,
    );
    // No × and no click-past: the only way out is one of the two buttons.
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    fireEvent.pointerDown(scrim());
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('leaves no body gap when there is no body', () => {
    render(<Modal open title="Nothing to say" onClose={() => {}} footer={<Button>OK</Button>} />);
    const tags = Array.from(screen.getByRole('dialog').children).map((el) => el.tagName);
    expect(tags).toEqual(['HEADER', 'FOOTER']);
  });

  it('does not offer a control that is not on screen', () => {
    render(
      <Modal open title="Advanced" onClose={() => {}}>
        <Button>Visible</Button>
        <Button>Also visible</Button>
        {/* Last in DOM order, so a wrap that ignored visibility would land here. */}
        <div hidden>
          <Button>Hidden</Button>
        </div>
      </Modal>,
    );
    const close = screen.getByRole('button', { name: 'Close' });
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: 'Tab', shiftKey: true });
    expect(screen.getByRole('button', { name: 'Also visible' })).toHaveFocus();
  });
});

describe('Drawer', () => {
  it('is the same window with a different position', () => {
    render(
      <Drawer open title="Settings" onClose={() => {}}>
        <input aria-label="Baud" />
      </Drawer>,
    );
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveAttribute('data-ui', 'drawer');
    expect(app().inert).toBe(true);
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();
  });
});

describe('Confirm', () => {
  it('focuses the action when the answer is just a yes', () => {
    render(<Confirm open title="Reset the session?" onConfirm={() => {}} onCancel={() => {}} />);
    expect(screen.getByRole('button', { name: 'Confirm' })).toHaveFocus();
  });

  it('focuses Cancel when the answer destroys something', () => {
    render(
      <Confirm
        open
        tone="danger"
        title="Delete 3 artifacts?"
        confirmLabel="Delete"
        onConfirm={() => {}}
        onCancel={() => {}}
      />,
    );
    // The rule the whole file is written for: a stray Enter must never destroy
    // anything, so the destructive button is the one that needs aiming.
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Delete' })).toHaveAttribute('data-tier', 'danger');
  });

  it('runs the answer, and Escape counts as a cancel', () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<Confirm open title="Overwrite?" onConfirm={onConfirm} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe('confirm()', () => {
  it('resolves a promise instead of three pieces of state', async () => {
    let opening!: Promise<boolean>;
    await act(async () => {
      opening = confirm({ title: 'Flash again?', tone: 'danger' });
    });
    const dialog = screen.getByRole('dialog');
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await expect(opening).resolves.toBe(false);
    await waitFor(() => expect(dialog).not.toBeInTheDocument());
    expect(document.querySelectorAll('[data-ui="modal"]')).toHaveLength(0);
    expect(app().inert).toBe(false);
  });

  it('resolves true on the confirm button', async () => {
    let opening!: Promise<boolean>;
    await act(async () => {
      opening = confirm({ title: 'Discard unsaved settings?', confirmLabel: 'Discard' });
    });
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    await expect(opening).resolves.toBe(true);
  });
});

describe('PopConfirm', () => {
  function Row() {
    const ref = useRef<HTMLButtonElement | null>(null);
    const [open, setOpen] = useState(false);
    return (
      <>
        <button type="button" ref={ref} onClick={() => setOpen((was) => !was)}>
          firmware.bin
        </button>
        <PopConfirm
          open={open}
          anchorRef={ref}
          onClose={() => setOpen(false)}
          onConfirm={() => {}}
          tone="danger"
          title="Delete this artifact?"
          message="It is the last build for this target."
          confirmLabel="Delete"
        />
      </>
    );
  }

  it('asks next to the row it is about, and Escape is not an answer', () => {
    render(<Row />);
    const trigger = screen.getByRole('button', { name: 'firmware.bin' });
    expect(screen.queryByRole('dialog')).toBeNull();

    trigger.focus();
    fireEvent.click(trigger);
    const panel = screen.getByRole('dialog', { name: 'Delete this artifact?' });
    expect(panel).toHaveAttribute('data-ui', 'popover');
    expect(panel).toHaveAttribute('data-padded');
    // Same rule as the modal: the destructive control is the one that needs aiming.
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();

    fireEvent.keyDown(panel, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(trigger).toHaveFocus();
  });
});

describe('Isolation', () => {
  it('is counted, so closing a nested panel does not unlock the app', () => {
    function Stacked() {
      const ref = useRef<HTMLButtonElement | null>(null);
      const [drawer, setDrawer] = useState(true);
      const [pop, setPop] = useState(true);
      return (
        <>
          <button type="button" ref={ref}>
            Row
          </button>
          <Drawer open={drawer} title="Settings" onClose={() => setDrawer(false)}>
            <Button>Remove</Button>
          </Drawer>
          <PopConfirm
            open={pop}
            anchorRef={ref}
            onClose={() => setPop(false)}
            onConfirm={() => setPop(false)}
            title="Remove this entry?"
            confirmLabel="Remove"
          />
        </>
      );
    }

    render(<Stacked />);
    expect(app().inert).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    // The popover is gone and the drawer is still up: the app is still behind a
    // scrim, so the keyboard must still be inside the drawer.
    expect(app().inert).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(app().inert).toBe(false);
  });
});
