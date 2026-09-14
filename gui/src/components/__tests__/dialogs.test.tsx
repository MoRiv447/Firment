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
 */

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
