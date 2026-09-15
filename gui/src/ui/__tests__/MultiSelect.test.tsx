import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { MultiSelect } from '../MultiSelect';

/**
 * The multi-select.
 *
 * Two of its behaviours are the reason it is not `Select` with a flag, and they
 * are the two this file spends most of its assertions on: the panel **stays open**
 * while you pick several, and typing **filters** rather than jumps.
 */

const OPTIONS = [
  { value: 'read_file', label: 'read_file' },
  { value: 'shell', label: 'shell' },
  { value: 'write_file', label: 'write_file' },
];

function setup(over: Partial<Parameters<typeof MultiSelect>[0]> = {}) {
  const onChange = vi.fn();
  render(
    <MultiSelect
      ariaLabel="Tools"
      options={OPTIONS}
      value={[]}
      onChange={onChange}
      placeholder="tool names"
      {...over}
    />,
  );
  const box = () => screen.getByRole('combobox', { name: 'Tools' }) as HTMLInputElement;
  const typeText = (text: string) => fireEvent.change(box(), { target: { value: text } });
  return { onChange, box, typeText };
}

describe('MultiSelect', () => {
  it('shows the placeholder only while nothing is chosen', () => {
    setup();
    expect(screen.getByPlaceholderText('tool names')).toBeInTheDocument();
  });

  it('shows a chip per chosen value and no placeholder', () => {
    setup({ value: ['read_file', 'shell'] });
    expect(screen.getByText('read_file')).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.queryByPlaceholderText('tool names')).toBeNull();
  });

  it('falls back to the raw value for a chip that is not in the options', () => {
    // A value can outlive its option: a tool gets renamed, a setting is loaded
    // from a config written by an older build.
    setup({ value: ['old_tool_name'] });
    expect(screen.getByText('old_tool_name')).toBeInTheDocument();
  });

  it('adds a value without closing the panel', () => {
    const { onChange, box } = setup();
    fireEvent.click(box());
    fireEvent.click(screen.getByRole('option', { name: 'shell' }));
    expect(onChange).toHaveBeenCalledWith(['shell']);
    // Still open: picking several is the entire point.
    expect(screen.getByRole('listbox')).toBeInTheDocument();
  });

  it('removes a value that is already chosen', () => {
    const { onChange, box } = setup({ value: ['shell'] });
    fireEvent.click(box());
    fireEvent.click(screen.getByRole('option', { name: 'shell' }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it('removes a chip from its own control', () => {
    const { onChange } = setup({ value: ['read_file', 'shell'] });
    fireEvent.click(screen.getByRole('button', { name: 'Remove read_file' }));
    expect(onChange).toHaveBeenCalledWith(['shell']);
  });

  it('filters the list as you type', () => {
    // creatable off, so the only row left is the one that matched.
    const { typeText, box } = setup({ creatable: false });
    fireEvent.click(box());
    typeText('sh');
    expect(screen.getAllByRole('option')).toHaveLength(1);
    expect(screen.getByRole('option', { name: 'shell' })).toBeInTheDocument();
  });

  it('says so when the filter matches nothing', () => {
    const { typeText, box } = setup({ creatable: false });
    fireEvent.click(box());
    typeText('zzz');
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(screen.getByText('No matches')).toBeInTheDocument();
  });

  it('takes back the last choice with Backspace on an empty box', () => {
    const { onChange, box } = setup({ value: ['read_file', 'shell'] });
    fireEvent.click(box());
    fireEvent.keyDown(box(), { key: 'Backspace' });
    expect(onChange).toHaveBeenCalledWith(['read_file']);
  });

  it('does NOT take anything back while there is text to delete', () => {
    const { onChange, box, typeText } = setup({ value: ['read_file'] });
    fireEvent.click(box());
    typeText('re');
    fireEvent.keyDown(box(), { key: 'Backspace' });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('toggles the highlighted row with Enter', () => {
    const { onChange, box } = setup();
    fireEvent.click(box());
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith(['read_file']);
  });

  it('offers what you typed when nothing matches it', () => {
    const { onChange, typeText, box } = setup();
    fireEvent.click(box());
    typeText('delay');
    const offer = screen.getByRole('option', { name: /Add .*delay/ });
    fireEvent.click(offer);
    expect(onChange).toHaveBeenCalledWith(['delay']);
  });

  it('clears the offer after taking it, so the list is a list again', () => {
    const { typeText, box } = setup({ value: ['delay'] });
    fireEvent.click(box());
    typeText('delay');
    fireEvent.click(screen.getByRole('option', { name: /Add .*delay/ }));
    // Careful: `onChange` is a spy, so the component's own value stays [].
    expect((box() as HTMLInputElement).value).toBe('');
  });

  it('does not offer a value that is already an option', () => {
    const { typeText, box } = setup();
    fireEvent.click(box());
    typeText('shell');
    expect(screen.queryByRole('option', { name: /^Add / })).toBeNull();
  });

  it('does not offer anything at all when it is not creatable', () => {
    const { typeText, box } = setup({ creatable: false });
    fireEvent.click(box());
    typeText('delay');
    expect(screen.queryByRole('option', { name: /^Add / })).toBeNull();
  });

  it('is a combobox over a multiselectable listbox, and marks the chosen rows', () => {
    const { box } = setup({ value: ['shell'] });
    fireEvent.click(box());
    expect(box()).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('listbox')).toHaveAttribute('aria-multiselectable', 'true');
    expect(screen.getByRole('option', { name: 'shell' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('option', { name: 'read_file' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('cannot be typed into or unpicked while disabled', () => {
    setup({ value: ['shell'], disabled: true });
    expect(screen.getByRole('combobox', { name: 'Tools' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Remove shell' })).toBeDisabled();
  });
});
