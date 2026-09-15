import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BranchDialog } from '../BranchDialog';

/**
 * The branch dialog.
 *
 * What it has to get right: the note has to name the session it branches from
 * (otherwise "new branch" is ambiguous the moment there is more than one
 * session), and the name you typed has to survive a refused create.
 */

function setup(over: Partial<Parameters<typeof BranchDialog>[0]> = {}) {
  const onCancel = vi.fn();
  const onCreate = vi.fn().mockResolvedValue(true);
  const view = render(
    <BranchDialog parentId="aaaaaaaa-1111-2222" busy={false} onCancel={onCancel} onCreate={onCreate} {...over} />,
  );
  const field = () => screen.getByRole('textbox', { name: 'branch title' }) as HTMLInputElement;
  return { onCancel, onCreate, view, field };
}

describe('BranchDialog', () => {
  it('is not on screen until a session is being branched from', () => {
    setup({ parentId: null });
    expect(screen.queryByText('New branch conversation')).toBeNull();
  });

  it('says which session this branch comes from', () => {
    setup();
    // The short id, because that is what the session tree shows.
    expect(screen.getByText(/aaaaaaaa/)).toBeInTheDocument();
    expect(screen.getByText(/inherits cwd\/provider\/model only/)).toBeInTheDocument();
  });

  it('will not create a nameless branch', () => {
    setup();
    expect(screen.getByRole('button', { name: 'create branch' })).toBeDisabled();
    fireEvent.change(screen.getByRole('textbox', { name: 'branch title' }), {
      target: { value: '   ' },
    });
    // Whitespace is not a name.
    expect(screen.getByRole('button', { name: 'create branch' })).toBeDisabled();
  });

  it('creates from the button, trimmed', async () => {
    const { onCreate, field } = setup();
    fireEvent.change(field(), { target: { value: '  sensor drift hunt  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'create branch' }));
    await waitFor(() => expect(onCreate).toHaveBeenCalledWith('sensor drift hunt'));
  });

  it('creates from Enter', async () => {
    const { onCreate, field } = setup();
    fireEvent.change(field(), { target: { value: 'sensor drift hunt' } });
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(onCreate).toHaveBeenCalledWith('sensor drift hunt'));
  });

  it('keeps the name when the branch was NOT created', async () => {
    const { field } = setup({ onCreate: vi.fn().mockResolvedValue(false) });
    fireEvent.change(field(), { target: { value: 'sensor drift hunt' } });
    fireEvent.click(screen.getByRole('button', { name: 'create branch' }));
    await waitFor(() => expect(field().value).toBe('sensor drift hunt'));
  });

  it('clears the name once the branch exists', async () => {
    const { field } = setup();
    fireEvent.change(field(), { target: { value: 'sensor drift hunt' } });
    fireEvent.click(screen.getByRole('button', { name: 'create branch' }));
    await waitFor(() => expect(field().value).toBe(''));
  });

  it('abandons the name on cancel, so a reopen does not look created', () => {
    const { onCancel, field } = setup();
    fireEvent.change(field(), { target: { value: 'sensor drift hunt' } });
    fireEvent.click(screen.getByRole('button', { name: 'cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(field().value).toBe('');
  });

  it('will not queue a second create from Enter while one is in flight', () => {
    const { onCreate, field } = setup({ busy: true });
    fireEvent.change(field(), { target: { value: 'sensor drift hunt' } });
    fireEvent.keyDown(field(), { key: 'Enter' });
    expect(onCreate).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'create branch' })).toBeDisabled();
  });
});
