import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { ProvidersCard } from '../ProvidersCard';
import type { ProviderEntryDto } from '../../../types';

/**
 * The provider list.
 *
 * The three write paths are the thing to pin, because two of them are easy to
 * get wrong in opposite directions: the row's fields must *not* write on every
 * keystroke (that is an RPC plus a full reload per character), and the add row
 * must not clear itself unless the provider was actually stored.
 */

const providers: ProviderEntryDto[] = [
  {
    name: 'deepseek',
    type: 'openai',
    base_url: 'https://api.deepseek.com/v1',
    model: 'deepseek-v4-flash',
    is_default: true,
    api_key: null,
  },
  {
    name: 'local',
    type: 'openai',
    base_url: null,
    model: 'qwen3',
    is_default: false,
    api_key: null,
  },
];

function setup(over: Partial<Parameters<typeof ProvidersCard>[0]> = {}) {
  const handlers = {
    onChange: vi.fn(),
    onPersist: vi.fn(),
    onSaveKey: vi.fn(),
    onRemove: vi.fn(),
    onAdd: vi.fn().mockResolvedValue(true),
  };
  render(
    <ProvidersCard providers={providers} newMsg="" keyMsg="" {...handlers} {...over} />,
  );
  const addRow = () => screen.getByPlaceholderText('name (e.g. deepseek)') as HTMLInputElement;
  const modelField = () => screen.getByPlaceholderText(/^model \(/) as HTMLInputElement;
  return { ...handlers, addRow, modelField };
}

describe('ProvidersCard', () => {
  it('lists every provider', () => {
    setup();
    expect(screen.getByText('deepseek')).toBeInTheDocument();
    expect(screen.getByText('local')).toBeInTheDocument();
  });

  it('marks only the default one', () => {
    setup();
    expect(screen.getAllByText('DEFAULT')).toHaveLength(1);
  });

  it('edits the row locally as you type, without writing', () => {
    const { onChange, onPersist } = setup();
    fireEvent.change(screen.getAllByPlaceholderText('base url')[0], {
      target: { value: 'https://x/v1' },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'deepseek' }),
      { base_url: 'https://x/v1' },
    );
    // The write waits for the blur.
    expect(onPersist).not.toHaveBeenCalled();
  });

  it('writes the row when the field is left', () => {
    const { onPersist } = setup();
    fireEvent.blur(screen.getAllByPlaceholderText('base url')[0]);
    expect(onPersist).toHaveBeenCalledWith(expect.objectContaining({ name: 'deepseek' }));
  });

  it('an emptied base url is null, not an empty string', () => {
    // `null` is what "no base url" means in the config; `''` would be written out
    // as an empty override.
    const { onChange } = setup();
    fireEvent.change(screen.getAllByPlaceholderText('base url')[0], { target: { value: '' } });
    expect(onChange).toHaveBeenCalledWith(expect.anything(), { base_url: null });
  });

  it('keeps the key draft in the same list as the rest of the row', () => {
    const { onChange } = setup();
    fireEvent.change(screen.getByPlaceholderText(/api key for deepseek/), {
      target: { value: 'sk-1' },
    });
    expect(onChange).toHaveBeenCalledWith(expect.anything(), { api_key: 'sk-1' });
  });

  it('saves the key with its own button, because it is its own file', () => {
    const { onSaveKey } = setup();
    fireEvent.click(screen.getAllByRole('button', { name: 'Save key' })[0]);
    expect(onSaveKey).toHaveBeenCalledWith(expect.objectContaining({ name: 'deepseek' }));
  });

  it('asks before deleting, and deletes when told to', async () => {
    const { onRemove } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Delete deepseek' }));
    expect(onRemove).not.toHaveBeenCalled();
    expect(screen.getByText(/Delete "deepseek"\?/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'delete' }));
    await waitFor(() => expect(onRemove).toHaveBeenCalledWith(expect.objectContaining({ name: 'deepseek' })));
  });

  it('does not delete when the question is dismissed', () => {
    const { onRemove } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Delete deepseek' }));
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(onRemove).not.toHaveBeenCalled();
  });

  it('will not add a provider without a name and a model', () => {
    setup();
    expect(screen.getByRole('button', { name: 'Save provider' })).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText('name (e.g. deepseek)'), {
      target: { value: 'glm' },
    });
    // A name on its own is not a provider.
    expect(screen.getByRole('button', { name: 'Save provider' })).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText(/^model \(/), { target: { value: 'glm-4' } });
    expect(screen.getByRole('button', { name: 'Save provider' })).toBeEnabled();
  });

  it('adds the provider it was given, trimmed, and clears the row', async () => {
    const { onAdd, addRow, modelField } = setup();
    fireEvent.change(addRow(), { target: { value: '  glm  ' } });
    fireEvent.change(screen.getByPlaceholderText(/base url \(e\.g\./), {
      target: { value: '  https://x/v1  ' },
    });
    fireEvent.change(modelField(), { target: { value: ' glm-4 ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save provider' }));

    await waitFor(() =>
      expect(onAdd).toHaveBeenCalledWith({
        name: 'glm',
        type: 'openai',
        baseUrl: 'https://x/v1',
        model: 'glm-4',
      }),
    );
    await waitFor(() => expect(addRow().value).toBe(''));
    expect(modelField().value).toBe('');
  });

  it('keeps what you typed when the provider was not stored', async () => {
    const { addRow, modelField } = setup({ onAdd: vi.fn().mockResolvedValue(false) });
    fireEvent.change(addRow(), { target: { value: 'glm' } });
    fireEvent.change(modelField(), { target: { value: 'glm-4' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save provider' }));
    await waitFor(() => expect(modelField().value).toBe('glm-4'));
    expect(addRow().value).toBe('glm');
  });

  it('adds from Enter in either of the two fields that must be filled', async () => {
    const { onAdd, addRow, modelField } = setup();
    fireEvent.change(addRow(), { target: { value: 'glm' } });
    fireEvent.change(modelField(), { target: { value: 'glm-4' } });
    fireEvent.keyDown(modelField(), { key: 'Enter' });
    await waitFor(() => expect(onAdd).toHaveBeenCalledTimes(1));
  });

  it('shows what the caller reported', () => {
    setup({ newMsg: 'saved provider "glm"', keyMsg: 'key saved for glm' });
    expect(screen.getByText('saved provider "glm"')).toBeInTheDocument();
    expect(screen.getByText('key saved for glm')).toBeInTheDocument();
  });
});
