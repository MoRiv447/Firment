import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { api } from '../../lib/api';
import { AskDialog, PermissionDialog } from '../Dialogs';
import type { AskRequest, PermissionRequest } from '../../types';

/**
 * What each dialog accepts as an answer, and what it refuses to treat as one.
 *
 * These two components are the only place the GUI sends a reply back into a turn
 * that is blocked waiting for it, so the behaviour worth pinning is the send, not
 * the markup: an answer that never reaches the kernel wedges the agent, and an
 * answer sent by a keystroke the user did not mean -- Escape, Enter, a second
 * click -- cannot be taken back.
 *
 * `api` is the real module with two of its methods spied on. Nothing here reaches
 * the network: the spy replaces the call before `invoke` is asked for a Tauri
 * runtime that jsdom does not have.
 *
 * The last case reads `App.tsx` as text, which is out of place here and deliberate:
 * what it checks is how the queue RENDERS these components, and every test in this
 * file renders one dialog on its own — the shape a queue gives them is invisible to
 * that, and invisible in a diff.
 */

// The app root, as text. Same route `ui/__tests__/conventions.test.ts` takes: the
// GUI has no Node types, so `?raw` is the only file read a test here can do.
const appSource = import.meta.glob('../../App.tsx', {
  query: '?raw',
  import: 'default',
  eager: true,
})['../../App.tsx'] as string;

const permission = (over: Partial<PermissionRequest> = {}): PermissionRequest => ({
  id: 7,
  tool: 'shell',
  args: { command: 'idf.py build' },
  reason: '',
  session_id: 's-1234567890abcdef',
  ...over,
});

const ask = (over: Partial<AskRequest> = {}): AskRequest => ({
  id: 9,
  question: 'Which board is wired up right now?',
  options: [],
  ...over,
});

const askSpy = vi.spyOn(api, 'respondAsk');
const permSpy = vi.spyOn(api, 'respondPermission');
const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

afterEach(() => {
  vi.clearAllMocks();
});

describe('PermissionDialog', () => {
  const open = (req = permission()) => {
    const onClose = vi.fn();
    render(<PermissionDialog req={req} onClose={onClose} />);
    return { onClose };
  };

  it('asks about the tool by name, and keeps the reason with it', () => {
    open(permission({ reason: 'This writes to COM5.' }));
    expect(screen.getByRole('dialog', { name: 'Permission requested' })).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.getByText('This writes to COM5.')).toBeInTheDocument();
  });

  it('names the chat that is asking, and carries its whole id', () => {
    open();
    const chip = screen.getByText('chat s-123456');
    expect(chip.closest('[data-ui="chip"]')).toHaveAttribute('title', 's-1234567890abcdef');
  });

  it('puts the caret on the answer that runs nothing', () => {
    open();
    expect(screen.getByRole('button', { name: 'Deny' })).toHaveFocus();
  });

  it('refuses to be dismissed: no close affordance, and Escape answers nothing', () => {
    const { onClose } = open();
    const panel = screen.getByRole('dialog');
    expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();

    fireEvent.keyDown(panel, { key: 'Escape' });
    expect(panel).toBeInTheDocument();
    expect(permSpy).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('sends Deny as false and Allow as true, and closes only once the kernel has it', async () => {
    permSpy.mockResolvedValue(undefined);
    const { onClose } = open();
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(permSpy).toHaveBeenCalledWith(7, true);
  });

  it('does not answer twice while the first answer is in flight', () => {
    permSpy.mockReturnValue(new Promise(() => {}));
    open();
    const allow = screen.getByRole('button', { name: 'Allow' });
    fireEvent.click(allow);
    fireEvent.click(allow);
    expect(permSpy).toHaveBeenCalledTimes(1);
  });

  it('stays actionable when the send fails, so the answer can be given again', async () => {
    permSpy.mockRejectedValueOnce(new Error('channel closed')).mockResolvedValue(undefined);
    const { onClose } = open();
    const allow = screen.getByRole('button', { name: 'Allow' });

    fireEvent.click(allow);
    await waitFor(() => expect(errorSpy).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
    expect(allow).toBeInTheDocument();

    fireEvent.click(allow);
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(permSpy).toHaveBeenCalledTimes(2);
  });
});

describe('AskDialog', () => {
  const open = (req = ask(), onClose = vi.fn()) => {
    render(<AskDialog req={req} onClose={onClose} />);
    return { onClose };
  };

  it('offers the options as the answers, and sends the one clicked', async () => {
    askSpy.mockResolvedValue(undefined);
    const { onClose } = open(ask({ options: ['ESP32-S3', 'nRF52840'] }));
    expect(screen.getByText('Which board is wired up right now?')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'nRF52840' }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(askSpy).toHaveBeenCalledWith(9, 'nRF52840');
  });

  it('offers no way to type an answer the kernel would not accept', () => {
    open(ask({ options: ['ESP32-S3'] }));
    expect(screen.queryByRole('textbox')).toBeNull();
  });

  it('starts in the answer box, not on the buttons', () => {
    open();
    expect(screen.getByRole('textbox')).toHaveFocus();
  });

  it('sends what was typed on Enter, without the newline that triggered it', async () => {
    askSpy.mockResolvedValue(undefined);
    const { onClose } = open();
    const field = screen.getByRole('textbox');
    fireEvent.change(field, { target: { value: 'the blue one' } });
    fireEvent.keyDown(field, { key: 'Enter' });

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(askSpy).toHaveBeenCalledWith(9, 'the blue one');
    expect(field).toHaveValue('the blue one');
  });

  it('treats Shift+Enter as a newline and not as an answer', () => {
    open();
    const field = screen.getByRole('textbox');
    fireEvent.change(field, { target: { value: 'line one' } });
    fireEvent.keyDown(field, { key: 'Enter', shiftKey: true });
    expect(askSpy).not.toHaveBeenCalled();
  });

  it('does not answer on a keystroke that belongs to the input method', () => {
    // This dialog is the one place a person types into a turn the agent is blocked on, and the
    // answer goes to the kernel the moment Enter is read as a submit. Confirming a candidate IS
    // an Enter: `isComposing` on the standard path, keyCode 229 on the engines that never raise
    // the flag when the composition ends. Both have to be refused, or the same field works in one
    // browser and answers a question with half a word in another.
    open();
    const field = screen.getByRole('textbox');
    fireEvent.change(field, { target: { value: 'na ge' } });
    fireEvent.keyDown(field, { key: 'Enter', isComposing: true });
    fireEvent.keyDown(field, { key: 'Enter', keyCode: 229 });
    expect(askSpy).not.toHaveBeenCalled();
    expect(field).toHaveValue('na ge');

    // Once the candidate is committed the same Enter is an answer again.
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(askSpy).toHaveBeenCalledWith(9, 'na ge');
  });
  it('will not send blank free text, and takes Dismiss as the explicit null', async () => {
    askSpy.mockResolvedValue(undefined);
    const { onClose } = open();
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(screen.getByRole('button', { name: 'Reply' })).toBeDisabled();
    expect(askSpy).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(askSpy).toHaveBeenCalledWith(9, null);
  });

  it('counts Escape as the dismissal it is here, unlike the permission prompt', () => {
    askSpy.mockResolvedValue(undefined);
    open();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(askSpy).toHaveBeenCalledWith(9, null);
  });

  it('does not answer twice while the first answer is in flight', () => {
    askSpy.mockReturnValue(new Promise(() => {}));
    open();
    const dismiss = screen.getByRole('button', { name: 'Dismiss' });
    fireEvent.click(dismiss);
    fireEvent.click(dismiss);
    expect(askSpy).toHaveBeenCalledTimes(1);
  });
});

describe('the queue that renders them', () => {
  it('actually reads the app root', () => {
    // A glob that matched nothing hands back `''`, and every assertion below would then be
    // vacuously true -- the same reason `conventions.test.ts` counts its own files.
    expect(appSource.length).toBeGreaterThan(1000);
  });

  it.each(['AskDialog', 'PermissionDialog'])(
    'renders %s with the request id as its key',
    (name) => {
      // Both dialogs keep state of their own: `AskDialog` the free text being typed,
      // `PermissionDialog` the flag that marks an answer in flight. The queue renders only
      // `queue[0]`, so answering one request while another waits swaps the props on the SAME
      // component instance -- unkeyed, the text typed for the first question is sitting in the
      // box when the second opens, and Enter sends it to a question it was never written for.
      // `id` is unique because the backend takes it from one process-global counter.
      const element = new RegExp(`<${name}\\b[\\s\\S]*?/>`).exec(appSource);
      expect(element, `${name} is no longer rendered from App.tsx`).toBeTruthy();
      expect(element![0], `${name} renders with no key`).toMatch(/key=\{/);
    },
  );
});
