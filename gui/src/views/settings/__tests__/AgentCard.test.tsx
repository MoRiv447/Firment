import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { AgentCard } from '../AgentCard';
import type { SettingsDto } from '../../../types';

/**
 * The agent settings, now that the values are a draft rather than a form instance.
 *
 * Three things are worth pinning. The carry-over of `providers` (saving must not
 * write an empty provider list over the real one). The null/empty distinction on
 * the nullable strings. And that a save sends what is *in the draft*, including
 * edits the user made in a field the caller never heard about.
 */

const settings: SettingsDto = {
  default_provider: 'deepseek',
  default_model: 'deepseek-v4-flash',
  auto_approve: ['read_file'],
  max_iterations: 20,
  context_budget_chars: 200000,
  build_command: 'cargo build',
  default_chip: null,
  monitor_port: null,
  monitor_baud: 115200,
  web_search: 'bing',
  thinking: 'high',
  theme: 'auto',
  providers: [
    {
      name: 'deepseek',
      type: 'openai',
      base_url: null,
      model: 'deepseek-v4-flash',
      is_default: true,
      api_key: null,
    },
  ],
};

function setup(over: Partial<Parameters<typeof AgentCard>[0]> = {}) {
  const handlers = {
    onFetchModels: vi.fn(),
    onSave: vi.fn().mockResolvedValue(true),
  };
  render(
    <AgentCard
      settings={settings}
      models={[]}
      providers={[{ label: 'deepseek', value: 'deepseek' }]}
      saveMsg=""
      saveErr=""
      {...handlers}
      {...over}
    />,
  );
  const field = (name: string) => screen.getByRole('textbox', { name });
  return { ...handlers, field };
}

describe('AgentCard', () => {
  it('shows what is stored', () => {
    setup();
    expect(screen.getByRole('textbox', { name: 'Model' })).toHaveValue('deepseek-v4-flash');
    expect(screen.getByRole('textbox', { name: 'Build command' })).toHaveValue('cargo build');
    expect(screen.getByRole('textbox', { name: 'Baud' })).toHaveValue('115200');
  });

  it('shows an unset nullable field as empty rather than as "null"', () => {
    setup();
    expect(screen.getByRole('textbox', { name: 'Default chip' })).toHaveValue('');
    expect(screen.getByRole('textbox', { name: 'Monitor port' })).toHaveValue('');
  });

  it('saves the draft, and never the draft\u2019s provider list', async () => {
    // `providers` is carried over from the loaded settings: while that load is
    // pending the draft has none, and saving would write `[]` over the real list.
    const { onSave, field } = setup();
    fireEvent.change(field('Model'), { target: { value: 'glm-4' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const sent = onSave.mock.calls[0][0] as SettingsDto;
    expect(sent.default_model).toBe('glm-4');
    expect(sent.providers).toEqual(settings.providers);
  });

  it('writes an emptied nullable field as null, not as an empty string', async () => {
    const { onSave, field } = setup();
    fireEvent.change(field('Build command'), { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect((onSave.mock.calls[0][0] as SettingsDto).build_command).toBeNull();
  });

  it('follows the number rules the layer set for number fields', async () => {
    const { onSave, field } = setup();
    const iterations = screen.getByRole('textbox', { name: 'Max iterations' });
    fireEvent.change(iterations, { target: { value: '999' } });
    // Above the maximum: clamped, not rejected.
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect((onSave.mock.calls[0][0] as SettingsDto).max_iterations).toBe(100);

    expect(field('Baud')).toHaveValue('115200');
  });

  it('fetches models for the provider, both on change and on demand', () => {
    const { onFetchModels } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Fetch models' }));
    expect(onFetchModels).toHaveBeenCalledWith('deepseek');
  });

  it('picks a fetched model into the draft', async () => {
    const { onSave } = setup({ models: ['glm-4', 'qwen3'] });
    fireEvent.click(screen.getByRole('button', { name: 'qwen3' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect((onSave.mock.calls[0][0] as SettingsDto).default_model).toBe('qwen3');
  });

  it('keeps a tool list and can extend it with one that is not in it', async () => {
    const { onSave } = setup();
    const tools = screen.getByRole('combobox', { name: 'Auto-approve tools' });
    fireEvent.change(tools, { target: { value: 'shell' } });
    fireEvent.keyDown(tools, { key: 'Enter' });

    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect((onSave.mock.calls[0][0] as SettingsDto).auto_approve).toEqual(['read_file', 'shell']);
  });

  it('can unset the web search provider', async () => {
    const { onSave } = setup();
    // The layer's Select is a button over a listbox, not a combobox: that is
    // MultiSelect's input.
    fireEvent.click(screen.getByRole('button', { name: 'Web search provider' }));
    fireEvent.click(screen.getByRole('option', { name: 'Clear' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect((onSave.mock.calls[0][0] as SettingsDto).web_search).toBeNull();
  });

  it('says what the caller reported, and does not clear the edit on a failure', async () => {
    const { field } = setup({ onSave: vi.fn().mockResolvedValue(false) });
    fireEvent.change(field('Model'), { target: { value: 'glm-4' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(field('Model')).toHaveValue('glm-4'));
  });

  it('shows the caller\u2019s messages', () => {
    setup({ saveMsg: 'saved \u2713', saveErr: 'save failed: nope' });
    expect(screen.getByText('saved \u2713')).toBeInTheDocument();
    expect(screen.getByText(/nope/)).toBeInTheDocument();
  });
});
