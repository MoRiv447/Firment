import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { CopyButton, Menu, Segmented, Select, Switch, Tabs } from '..';
import type { MenuEntry } from '..';

/**
 * What the keyboard does to the pickers.
 *
 * These four controls are the ones a hand-rolled implementation usually gets
 * wrong, because the mistakes are invisible to a mouse: a cursor that parks on a
 * disabled row, an Escape that closes the window behind the panel, a Tab that
 * throws focus to the top of the document. Each test here is one of those.
 */

const OPTIONS = [
  { value: 'alpha', label: 'Alpha' },
  { value: 'bravo', label: 'Bravo' },
  { value: 'charlie', label: 'Charlie', disabled: true },
  { value: 'delta', label: 'Delta' },
];

function Picker({ onChange }: { onChange: (value: string) => void }) {
  const [value, setValue] = useState<string | undefined>('bravo');
  return (
    <Select
      ariaLabel="Model"
      options={OPTIONS}
      value={value}
      onChange={(next) => {
        setValue(next);
        onChange(next);
      }}
    />
  );
}

describe('Select', () => {
  it('opens from the trigger and starts the cursor on the current value', () => {
    render(<Picker onChange={() => {}} />);
    const trigger = screen.getByRole('button', { name: 'Model' });
    fireEvent.click(trigger);
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    // Row 1 is `bravo`: pressing Enter now must change nothing.
    expect(trigger.getAttribute('aria-activedescendant')).toMatch(/-1$/);
  });

  it('walks past a disabled option instead of parking on it', () => {
    render(<Picker onChange={() => {}} />);
    const trigger = screen.getByRole('button', { name: 'Model' });
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    expect(trigger.getAttribute('aria-activedescendant')).toMatch(/-3$/);
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    expect(trigger.getAttribute('aria-activedescendant')).toMatch(/-0$/);
  });

  it('commits with Enter and keeps focus on the trigger', () => {
    const onChange = vi.fn();
    render(<Picker onChange={onChange} />);
    const trigger = screen.getByRole('button', { name: 'Model' });
    // The combobox owns its own focus: the panel is never a tab stop, so the
    // arrow keys act on `aria-activedescendant` and Enter closes back to here.
    trigger.focus();
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith('bravo');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it('does not typeahead onto a disabled row', () => {
    render(<Picker onChange={() => {}} />);
    const trigger = screen.getByRole('button', { name: 'Model' });
    fireEvent.click(trigger);
    // 'c' would reach `charlie`, and its label is blanked for typeahead, so
    // nothing matches and the cursor must stay where it was rather than land on
    // a row the user cannot choose.
    fireEvent.keyDown(trigger, { key: 'c' });
    expect(trigger.getAttribute('aria-activedescendant')).toMatch(/-1$/);
  });

  it('swallows Escape instead of letting it close the window behind it', () => {
    const behind = vi.fn();
    render(
      <div onKeyDown={behind}>
        <Picker onChange={() => {}} />
      </div>,
    );
    const trigger = screen.getByRole('button', { name: 'Model' });
    fireEvent.click(trigger);
    // `fireEvent` returns false once the event has been prevented.
    expect(fireEvent.keyDown(trigger, { key: 'Escape' })).toBe(false);
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(behind).not.toHaveBeenCalled();
  });

  it('commits on Tab rather than cancelling', () => {
    const onChange = vi.fn();
    render(<Picker onChange={onChange} />);
    const trigger = screen.getByRole('button', { name: 'Model' });
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: 'Tab' });
    expect(onChange).toHaveBeenCalledWith('bravo');
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});

const ITEMS: MenuEntry[] = [
  { key: 'a', label: 'Alpha' },
  { separator: true, key: 'sep' },
  { key: 'b', label: 'Bravo' },
  { key: 'dead', label: 'Unavailable', disabled: true },
];

function Host({ onSelect }: { onSelect: () => void }) {
  const ref = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" ref={ref} onClick={() => setOpen((was) => !was)}>
        Commands
      </button>
      <Menu
        open={open}
        anchorRef={ref}
        onClose={() => setOpen(false)}
        items={ITEMS.map((entry) =>
          entry.key === 'a' || entry.key === 'b' ? { ...entry, onSelect } : entry,
        )}
      />
    </>
  );
}

describe('Menu', () => {
  it('moves focus into the panel when it opens', () => {
    render(<Host onSelect={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Commands' }));
    expect(screen.getByRole('menu')).toHaveFocus();
  });

  it('counts rows the arrows can reach, so a separator is not a keypress', () => {
    render(<Host onSelect={() => {}} />);
    const trigger = screen.getByRole('button', { name: 'Commands' });
    fireEvent.click(trigger);
    const menu = screen.getByRole('menu');
    expect(screen.getAllByRole('menuitem')).toHaveLength(3);
    // One step lands on Bravo, skipping the separator; the next skips the dead row
    // and wraps to the top.
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(screen.getByRole('menuitem', { name: /Bravo/ })).toHaveAttribute(
      'data-active',
      'true',
    );
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(screen.getByRole('menuitem', { name: /Alpha/ })).toHaveAttribute(
      'data-active',
      'true',
    );
  });

  it('runs the row, closes, and hands focus back to the trigger', () => {
    const onSelect = vi.fn();
    render(<Host onSelect={onSelect} />);
    const trigger = screen.getByRole('button', { name: 'Commands' });
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it('closes on Escape back to the trigger', () => {
    render(<Host onSelect={() => {}} />);
    const trigger = screen.getByRole('button', { name: 'Commands' });
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });
});

describe('Segmented', () => {
  /** Controlled, so the test has to hold the value for the ring to follow. */
  function ViewSwitch({ onChange }: { onChange: (value: string) => void }) {
    const [value, setValue] = useState('chat');
    return (
      <Segmented
        ariaLabel="View"
        value={value}
        onChange={(next) => {
          setValue(next);
          onChange(next);
        }}
        options={[
          { value: 'chat', label: 'Chat' },
          { value: 'changes', label: 'Changes' },
        ]}
      />
    );
  }

  it('is one tab stop whose arrows move the value, not just the caret', () => {
    const onChange = vi.fn();
    render(<ViewSwitch onChange={onChange} />);
    const group = screen.getByRole('radiogroup', { name: 'View' });
    const radios = screen.getAllByRole('radio');
    expect(radios.map((r) => r.getAttribute('tabindex'))).toEqual(['0', '-1']);
    fireEvent.keyDown(group, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledWith('changes');
    expect(radios[1]).toHaveFocus();
    expect(radios[1]).toHaveAttribute('aria-checked', 'true');
    expect(radios[0]).toHaveAttribute('aria-checked', 'false');
  });
});

describe('Tabs', () => {
  const items = [
    { key: 'one', label: 'One' },
    { key: 'two', label: 'Two' },
    { key: 'dead', label: 'Dead', disabled: true },
    { key: 'four', label: 'Four' },
  ];

  it('skips a tab that cannot answer and wraps at the end', () => {
    const onChange = vi.fn();
    render(<Tabs ariaLabel="Inspector" items={items} active="two" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Two' }), { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledWith('four');
    expect(screen.getByRole('tab', { name: 'Four' })).toHaveFocus();
  });

  it('names the panel it controls', () => {
    render(
      <Tabs ariaLabel="Inspector" items={items} active="one" onChange={() => {}} idPrefix="insp" />,
    );
    expect(screen.getByRole('tab', { name: 'One' })).toHaveAttribute(
      'aria-controls',
      'insp-panel-one',
    );
  });
});

describe('Switch', () => {
  it('takes its accessible name from `name` when the label is elsewhere', () => {
    const onChange = vi.fn();
    render(<Switch checked={false} onChange={onChange} name="Verbose" />);
    fireEvent.click(screen.getByRole('switch', { name: 'Verbose' }));
    expect(onChange).toHaveBeenCalledWith(true);
  });
});

describe('CopyButton', () => {
  it('writes the value and answers on the button itself', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    render(<CopyButton value="target/debug/firment.elf" />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(writeText).toHaveBeenCalledWith('target/debug/firment.elf');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Copied' })).toBeInTheDocument(),
    );
  });

  it('says so when the clipboard refuses', async () => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
      configurable: true,
    });
    render(<CopyButton value="x" />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Copy failed' })).toBeInTheDocument(),
    );
  });
});
