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
    has_key: true,
    key_source: 'configured (auth.json)',
  },
  {
    name: 'local',
    type: 'openai',
    base_url: null,
    model: 'qwen3',
    is_default: false,
    has_key: false,
    key_source: 'MISSING (no api_key or api_key_env)',
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
    <ProvidersCard providers={providers} newMsg={null} keyMsg={null} {...handlers} {...over} />,
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

  it('keeps a stored key out of the field and the draft out of the settings', () => {
    // The box used to be pre-filled from `get_settings`, which meant the live key travelled to a
    // window whose only job is to display settings and sat in its DOM. The field is write-only, so
    // what it needs back is `has_key` and where the key came from -- not the value.
    const { onChange, onSaveKey } = setup();
    const box = () =>
      screen.getByPlaceholderText(/api key for deepseek/) as HTMLInputElement;
    expect(box().value).toBe('');
    expect(screen.queryAllByText('configured (auth.json)')).toHaveLength(1);

    fireEvent.change(box(), { target: { value: 'sk-1' } });
    // Not a patch on the settings list: a key is its own file, and the draft is the row's.
    expect(onChange).not.toHaveBeenCalled();
    expect(onSaveKey).not.toHaveBeenCalled();

    fireEvent.click(screen.getAllByRole('button', { name: 'Save key' })[0]);
    expect(onSaveKey).toHaveBeenCalledWith(expect.objectContaining({ name: 'deepseek' }), 'sk-1');
    expect(box().value).toBe('');
  });

  it('will not offer to save a key that has not been typed', () => {
    // This used to be a click that answered "empty key — nothing saved". A button that is off is
    // the same information before the fact rather than after it.
    setup();
    const save = screen.getAllByRole('button', { name: 'Save key' })[0] as HTMLButtonElement;
    expect(save.disabled).toBe(true);
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
    setup({
      newMsg: { text: 'saved provider "glm"', tone: 'ok' },
      keyMsg: { text: 'key saved for glm', tone: 'ok' },
    });
    expect(screen.getByText('saved provider "glm"')).toBeInTheDocument();
    expect(screen.getByText('key saved for glm')).toBeInTheDocument();
  });

  it('draws a failed write as a failure, not as a save', () => {
    // The colour was the defect: `failed: …` and `key saved for …` both arrived as bare
    // strings and both were painted in `--success-ink`, so the line reporting the refusal was
    // drawn in the colour of the thing it says did not happen.
    //
    // One render, one failure and one success side by side, and their class names compared to
    // each other rather than to a literal: CSS modules scope the names, and what is being
    // claimed is only that the two tones are not the same style.
    setup({
      newMsg: { text: 'could not save "glm": refused', tone: 'error' },
      keyMsg: { text: 'key saved for glm', tone: 'ok' },
    });
    const failure = screen.getByText('could not save "glm": refused');
    const success = screen.getByText('key saved for glm');
    expect(failure).toBeInTheDocument();
    expect(failure.className).not.toEqual(success.className);
    // The other half of the same sentence: a row-level failure has to reach the screen at all.
    // `editProvider` and `removeProvider` used to end their catch in `console.error`, which is
    // invisible to anyone without devtools open.
    expect(success).toBeInTheDocument();
  });
});
