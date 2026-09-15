import { X } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';

import { cx } from './cx';
import frame from './control.module.css';
import rows from './floating.module.css';
import { Icon } from './Icon';
import type { SelectOption } from './Select';
import { Popover } from './Popover';
import styles from './MultiSelect.module.css';
import type { Size } from './types';
import { useFieldProps } from './Field';
import { useListKeyboard } from './useListKeyboard';

/** A row that is not in `options`: the text you typed, offered back to you. */
type Row = SelectOption & { create?: boolean };

/** What to filter and to show for a row whose label is not a plain string. */
function labelText(option: SelectOption): string {
  return typeof option.label === 'string' ? option.label : option.value;
}

/**
 * A list you pick several things from, and type new ones into.
 *
 * It is a separate component rather than `Select` with a `multiple` flag, because
 * two of the three things that make it different are not about the list:
 *
 *   * **the trigger is an input.** The value is a set, so the box shows chips and
 *     keeps a text field for filtering -- the same element that takes focus, which
 *     is why the panel is anchored to the frame rather than to a button.
 *   * **the listbox is `aria-multiselectable` and Enter toggles rather than
 *     commits.** In `Select`, Enter means "take this and close"; here it means
 *     "add this one too", and the panel stays open. One control cannot honestly
 *     mean both, and a `value: string | string[]` union would have made every
 *     existing single-select caller's types worse to express the difference.
 *
 * Typing filters. `creatable` additionally offers what you typed as a row of its
 * own when nothing matches it exactly, which is what makes this usable for a list
 * the user extends rather than one they choose from -- tool names, for instance.
 */
export function MultiSelect({
  options,
  value,
  onChange,
  placeholder,
  size = 'md',
  disabled = false,
  invalid = false,
  creatable = true,
  emptyLabel = 'No matches',
  id,
  ariaLabel,
}: {
  options: SelectOption[];
  value: string[];
  onChange: (next: string[]) => void;
  /** Shown in the field while nothing is chosen. Not a default value. */
  placeholder?: string;
  size?: Size;
  disabled?: boolean;
  invalid?: boolean;
  /** Offer the typed text as a new value when nothing matches it exactly. */
  creatable?: boolean;
  emptyLabel?: string;
  id?: string;
  ariaLabel?: string;
}) {
  const base = useId();
  const listboxId = `${base}-listbox`;
  const anchorRef = useRef<HTMLSpanElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(-1);
  const [query, setQuery] = useState('');

  const a11y = useFieldProps({ id, invalid });
  const chosen = useMemo(() => new Set(value), [value]);

  const items = useMemo<Row[]>(() => {
    const needle = query.trim().toLowerCase();
    const matches =
      needle === ''
        ? options
        : options.filter((option) => labelText(option).toLowerCase().includes(needle));
    const typed = query.trim();
    if (!creatable || typed === '' || options.some((option) => option.value === typed)) {
      return matches;
    }
    return [...matches, { value: typed, label: `Add “${typed}”`, create: true }];
  }, [options, query, creatable]);

  const toggle = (v: string) => {
    onChange(chosen.has(v) ? value.filter((x) => x !== v) : [...value, v]);
  };

  const pick = (index: number) => {
    const row = items[index];
    if (!row) return;
    if (row.create) {
      if (!chosen.has(row.value)) onChange([...value, row.value]);
      setQuery('');
      setCursor(-1);
      return;
    }
    toggle(row.value);
  };

  const listKeys = useListKeyboard({
    count: items.length,
    active: cursor,
    onActive: setCursor,
    onCommit: pick,
    onClose: () => setOpen(false),
    // No `labels`: typeahead would fight the filter, since every keystroke is
    // already narrowing the list.
  });

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // Backspace on an empty query is how you take back the last choice -- the
    // usual gesture for this control, and the only keyboard way to remove one
    // without arrowing to it.
    if (event.key === 'Backspace' && query === '' && value.length > 0) {
      event.preventDefault();
      onChange(value.slice(0, -1));
      return;
    }
    if (!open) {
      if (event.key === 'ArrowDown' || event.key === 'Enter') {
        event.preventDefault();
        setOpen(true);
      }
      return;
    }
    listKeys(event);
  };

  // The highlighted row has to be visible in a list longer than the panel.
  useEffect(() => {
    if (!open || cursor < 0) return;
    document.getElementById(`${base}-${cursor}`)?.scrollIntoView?.({ block: 'nearest' });
  }, [open, cursor, base]);

  return (
    <>
      <span
        ref={anchorRef}
        data-ui="multiselect"
        data-size={size}
        data-open={open || undefined}
        data-disabled={disabled || undefined}
        data-invalid={a11y['aria-invalid'] ? 'true' : undefined}
        className={cx(frame.frame, frame.trigger, styles.frame)}
        onClick={() => inputRef.current?.focus()}
      >
        {value.map((v) => (
          <span key={v} className={styles.chip}>
            <span className={styles.chipText}>
              {labelText(options.find((o) => o.value === v) ?? { value: v, label: v })}
            </span>
            <button
              type="button"
              aria-label={`Remove ${v}`}
              disabled={disabled}
              className={styles.remove}
              onClick={(event) => {
                event.stopPropagation();
                onChange(value.filter((x) => x !== v));
              }}
            >
              <Icon src={X} tone="muted" />
            </button>
          </span>
        ))}
        <input
          {...a11y}
          ref={inputRef}
          data-ui="input"
          role="combobox"
          aria-label={ariaLabel}
          aria-expanded={open}
          aria-controls={open ? listboxId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={open && cursor >= 0 ? `${base}-${cursor}` : undefined}
          disabled={disabled}
          value={query}
          placeholder={value.length === 0 ? placeholder : undefined}
          className={styles.field}
          onChange={(event) => {
            setQuery(event.target.value);
            setCursor(-1);
            setOpen(true);
          }}
          // Clicking or tabbing into the field opens the list: it is a combobox,
          // and a combobox that needs a keystroke before it shows you anything
          // reads as an empty text box.
          onFocus={() => setOpen(true)}
          onKeyDown={onKeyDown}
        />
      </span>

      <Popover
        open={open && !disabled}
        anchorRef={anchorRef}
        onClose={() => setOpen(false)}
        role="listbox"
        multiselectable
        id={listboxId}
        matchAnchorWidth
        closeOnScroll={false}
      >
        {items.map((row, index) => (
          <div
            key={row.create ? '__create' : row.value}
            id={`${base}-${index}`}
            data-ui="option"
            role="option"
            aria-selected={row.create ? undefined : chosen.has(row.value)}
            data-active={index === cursor || undefined}
            data-selected={!row.create && chosen.has(row.value) ? true : undefined}
            data-create={row.create || undefined}
            className={cx(rows.item, styles.option)}
            onPointerEnter={() => setCursor(index)}
            onClick={() => pick(index)}
          >
            <span className={styles.text}>{row.label}</span>
          </div>
        ))}
        {items.length === 0 && <p className={styles.empty}>{emptyLabel}</p>}
      </Popover>
    </>
  );
}
