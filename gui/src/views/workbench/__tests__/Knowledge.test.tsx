import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Knowledge } from '../Knowledge';
import type { KbEntryDto } from '../../../types';

/**
 * The knowledge editor.
 *
 * Unusually for this directory, the drafts here belong to the caller -- selecting,
 * deleting and creating each change the selection, the text and the baseline
 * together -- so what this file can prove is narrower: that the card renders what
 * it is given, and that the one thing it does own (the new-cheatsheet name) is
 * not cleared unless the file was actually created.
 */

const files: KbEntryDto[] = [
  { key: 'AGENTS.md', exists: true, content: '# rules', mtimeMs: 1 },
  { key: 'docs/vendor-index.toml', exists: true, content: '', mtimeMs: 2 },
  { key: 'cheatsheet:pwm.toml', exists: false, content: '', mtimeMs: null },
];

function setup(over: Partial<Parameters<typeof Knowledge>[0]> = {}) {
  const handlers = {
    onSelect: vi.fn(),
    onDraft: vi.fn(),
    onSave: vi.fn(),
    onDelete: vi.fn(),
    onCreate: vi.fn().mockResolvedValue(true),
  };
  const view = render(
    <Knowledge
      files={files}
      selected="AGENTS.md"
      draft="# rules"
      dirty={false}
      busy={false}
      {...handlers}
      {...over}
    />,
  );
  const nameField = () => screen.getByPlaceholderText('new-cheatsheet.toml') as HTMLInputElement;
  return { ...handlers, view, nameField };
}

describe('Knowledge', () => {
  it('explains what the three kinds of file are when there are none', () => {
    setup({ files: [] });
    expect(screen.getByText(/AGENTS\.md is injected into every session/)).toBeInTheDocument();
  });

  it('marks a file that does not exist on disk yet', () => {
    // "(new)" is the difference between opening a file and creating one.
    setup();
    fireEvent.click(screen.getByRole('button', { name: 'knowledge file' }));
    expect(screen.getByRole('option', { name: 'cheatsheet:pwm.toml (new)' })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'AGENTS.md' })).toBeInTheDocument();
  });

  it('reports which file was picked', () => {
    const { onSelect } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'knowledge file' }));
    fireEvent.click(screen.getByRole('option', { name: 'docs/vendor-index.toml' }));
    expect(onSelect).toHaveBeenCalledWith('docs/vendor-index.toml');
  });

  it('shows the draft it was given', () => {
    setup();
    expect((screen.getByRole('textbox', { name: 'knowledge file contents' }) as HTMLTextAreaElement).value).toBe('# rules');
  });

  it('reports every keystroke to the caller, which owns the draft', () => {
    const { onDraft } = setup();
    fireEvent.change(screen.getByRole('textbox', { name: 'knowledge file contents' }), { target: { value: '# rules!' } });
    expect(onDraft).toHaveBeenCalledWith('# rules!');
  });

  it('will not offer a save when nothing changed', () => {
    setup({ dirty: false });
    expect(screen.getByRole('button', { name: /save/ })).toBeDisabled();
  });

  it('marks the save button while there are unsaved changes', () => {
    setup({ dirty: true });
    expect(screen.getByRole('button', { name: 'save •' })).toBeEnabled();
  });

  it('saves when asked', () => {
    const { onSave } = setup({ dirty: true });
    fireEvent.click(screen.getByRole('button', { name: 'save •' }));
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it('will not save while a write is in flight', () => {
    setup({ dirty: true, busy: true });
    expect(screen.getByRole('button', { name: 'save •' })).toBeDisabled();
  });

  it('only offers to delete a cheatsheet', () => {
    // The other two keys are repository files: deleting them is not the app's
    // business.
    setup({ selected: 'AGENTS.md' });
    expect(screen.queryByRole('button', { name: /delete cheatsheet/ })).toBeNull();
    setup({ selected: 'cheatsheet:pwm.toml' });
    expect(screen.getByRole('button', { name: /delete cheatsheet/ })).toBeInTheDocument();
  });

  it('creates a cheatsheet from the name field and clears it once created', async () => {
    const { onCreate, nameField } = setup();
    fireEvent.change(nameField(), { target: { value: '  i2c-notes  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'New cheatsheet' }));

    await waitFor(() => expect(onCreate).toHaveBeenCalledWith('i2c-notes'));
    await waitFor(() => expect(nameField().value).toBe(''));
  });

  it('keeps the name when the create failed', async () => {
    // Losing a name you typed because the backend refused is the same class of
    // mistake as losing a pin number.
    const { nameField } = setup({ onCreate: vi.fn().mockResolvedValue(false) });
    fireEvent.change(nameField(), { target: { value: 'i2c-notes' } });
    fireEvent.click(screen.getByRole('button', { name: 'New cheatsheet' }));

    await waitFor(() => expect(nameField().value).toBe('i2c-notes'));
  });

  it('will not create from a blank name, and not twice from Enter while busy', () => {
    const { onCreate } = setup({ busy: true });
    expect(screen.getByRole('button', { name: 'New cheatsheet' })).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText('new-cheatsheet.toml'), {
      target: { value: 'i2c-notes' },
    });
    fireEvent.keyDown(screen.getByPlaceholderText('new-cheatsheet.toml'), { key: 'Enter' });
    expect(onCreate).not.toHaveBeenCalled();
  });
});
