import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { Select } from '../Select';

/**
 * The clear row.
 *
 * The single-select behaviour has its own file (`controls.test.tsx`); what is
 * added here is the fourth row, and the two ways it can be wrong: appearing when
 * there is nothing to clear, and making the trigger read "Clear" instead of the
 * placeholder after the value has gone.
 */

const OPTIONS = [
  { value: 'bing', label: 'bing (default)' },
  { value: 'duckduckgo', label: 'duckduckgo' },
];

function setup(over: Partial<Parameters<typeof Select>[0]> = {}) {
  const onChange = vi.fn();
  render(
    <Select ariaLabel="Provider" options={OPTIONS} value="bing" onChange={onChange} {...over} />,
  );
  const trigger = () => screen.getByRole('button', { name: 'Provider' });
  const open = () => fireEvent.click(trigger());
  return { onChange, trigger, open };
}

describe('Select clearable', () => {
  it('offers no way out when it was not asked for', () => {
    const { open } = setup();
    open();
    expect(screen.getAllByRole('option')).toHaveLength(2);
  });

  it('adds one row when there is something to clear', () => {
    const { open } = setup({ clearable: true });
    open();
    expect(screen.getAllByRole('option')).toHaveLength(3);
    expect(screen.getByRole('option', { name: 'Clear' })).toBeInTheDocument();
  });

  it('takes the label it was given', () => {
    const { open } = setup({ clearable: true, clearLabel: 'Use the default' });
    open();
    expect(screen.getByRole('option', { name: 'Use the default' })).toBeInTheDocument();
  });

  it('clears by emitting the empty string, not undefined', () => {
    // `undefined` would make every existing caller handle a second absent value;
    // `''` is what the call sites already store for "unset".
    const { onChange, open } = setup({ clearable: true });
    open();
    fireEvent.click(screen.getByRole('option', { name: 'Clear' }));
    expect(onChange).toHaveBeenCalledWith('');
  });

  it('closes once it is cleared', () => {
    const { open } = setup({ clearable: true });
    open();
    fireEvent.click(screen.getByRole('option', { name: 'Clear' }));
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('shows the placeholder, not the word "Clear", once the value is gone', () => {
    // The clear row must not be able to look like the chosen value. Rendered on
    // its own: a second render in the same test leaves two selects on the page,
    // and every count after that is wrong.
    render(
      <Select
        ariaLabel="Cleared"
        options={OPTIONS}
        value=""
        clearable
        placeholder="nothing set"
        onChange={() => {}}
      />,
    );
    expect(screen.getByRole('button', { name: 'Cleared' })).toHaveTextContent('nothing set');
  });

  it('offers no way out when the value is already the empty string, and none when it is absent', () => {
    render(<Select ariaLabel="Empty" options={OPTIONS} value="" clearable onChange={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Empty' }));
    // '' means unset, so there is nothing to clear.
    expect(screen.getAllByRole('option')).toHaveLength(2);
    fireEvent.keyDown(screen.getByRole('button', { name: 'Empty' }), { key: 'Escape' });

    render(
      <Select ariaLabel="Absent" options={OPTIONS} value={undefined} clearable onChange={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Absent' }));
    expect(screen.getAllByRole('option')).toHaveLength(2);
  });

  it('can be reached by keyboard, as the last row', () => {
    const { onChange, open, trigger } = setup({ clearable: true });
    open();
    // Down from the current value ('bing', row 0) walks to the second option and
    // then to Clear; the cursor starts on the selected row, not the top.
    fireEvent.keyDown(trigger(), { key: 'ArrowDown' });
    fireEvent.keyDown(trigger(), { key: 'ArrowDown' });
    fireEvent.keyDown(trigger(), { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith('');
  });
});
