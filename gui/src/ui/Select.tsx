import { ChevronDown } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';

import { cx } from './cx';
import frame from './control.module.css';
import rows from './floating.module.css';
import { Icon } from './Icon';
import { Popover } from './Popover';
import styles from './Select.module.css';
import type { Size } from './types';
import { useFieldProps } from './Field';
import { useListKeyboard } from './useListKeyboard';

/** A row in a list: the value it writes, and what to show for it. */
export interface SelectOption {
  value: string;
  label: ReactNode;
  /** A second line: what picking this costs, or where it came from. */
  hint?: string;
  disabled?: boolean;
}

/**
 * A single-choice dropdown.
 *
 * The ARIA model is a `button` that opens a `listbox`, with `aria-activedescendant`
 * on the *button*: focus never moves into the panel, which is what lets a `Select`
 * sit inside a `Drawer` without the two fighting over the caret, and what keeps the
 * trigger's own `:focus-visible` ring where the user can see it.
 *
 * Value stays controlled. The app's settings write through on every change, so an
 * internal draft value would be a second copy of a number that has to be
 * synchronised with the server.
 *
 * "Clear" is an option with an empty value rather than a hover-revealed `×`: a
 * control that only appears when you mouse near it is how a setting ends up being
 * unset by accident.
 */
export function Select({
  options,
  value,
  onChange,
  placeholder,
  size = 'md',
  disabled = false,
  invalid = false,
  mono = false,
  clearable = false,
  clearLabel = 'Clear',
  id,
  ariaLabel,
}: {
  options: SelectOption[];
  value: string | undefined;
  onChange: (value: string) => void;
  /** What to show when nothing is chosen yet. Not a default value. */
  placeholder?: string;
  size?: Size;
  disabled?: boolean;
  /** Paired with `Field`'s `error`: the frame reddens and `aria-invalid` lands. */
  invalid?: boolean;
  /** The chosen value is a path, a port or a model id, so it reads better in mono. */
  mono?: boolean;
  /**
   * Offer a row that unsets the value, as the *last row of the list* rather than
   * a `×` that appears on hover.
   *
   * The row emits the empty string, which is why this does not widen `onChange`:
   * the call sites already treat `''` as "unset" -- the select's own value is
   * `string | undefined` and the field that writes it stores `''`. `undefined`
   * would have made every existing caller handle a second absent value.
   */
  clearable?: boolean;
  clearLabel?: string;
  id?: string;
  ariaLabel?: string;
}) {
  const base = useId();
  const listboxId = `${base}-listbox`;
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(-1);

  const a11y = useFieldProps({ id, invalid });

  // `''` is how a cleared select reads, so it is not a chosen value: the
  // placeholder belongs there, not the word "Clear".
  const cleared = value === undefined || value === '';

  const items = useMemo(() => {
    // No row when there is nothing to clear: an option that does nothing when
    // picked is worse than no option.
    if (!clearable || cleared) return options;
    return [...options, { value: '', label: clearLabel }];
  }, [options, clearable, clearLabel, cleared]);

  const selectedIndex = useMemo(
    () => (cleared ? -1 : items.findIndex((option) => option.value === value)),
    [items, value, cleared],
  );

  // A disabled row keeps an empty label rather than its own: typeahead matches by
  // prefix, so "" is a row the keyboard cannot land on -- which is the same
  // answer the pointer gets, where the row is painted dead and Enter refuses it.
  const labels = useMemo(
    () =>
      items.map((option) =>
        option.disabled ? '' : typeof option.label === 'string' ? option.label : '',
      ),
    [items],
  );

  // Start the cursor on the current value, not on the top of the list: a user who
  // opens a picker and presses Enter should not have just changed the setting.
  useEffect(() => {
    if (open) setCursor(selectedIndex);
  }, [open, selectedIndex]);

  const commit = (index: number) => {
    const option = items[index];
    if (!option || option.disabled) return;
    onChange(option.value);
    setOpen(false);
  };

  const listKeys = useListKeyboard({
    count: items.length,
    active: cursor,
    onActive: setCursor,
    onCommit: commit,
    onClose: () => setOpen(false),
    labels,
    isEnabled: (index) => !items[index]?.disabled,
  });

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!open) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        setOpen(true);
      }
      return;
    }
    listKeys(event);
  };

  // The highlighted option has to be the visible one, on a list of model ids that
  // is longer than the panel.
  useEffect(() => {
    if (!open || cursor < 0) return;
    document.getElementById(`${base}-${cursor}`)?.scrollIntoView?.({ block: 'nearest' });
  }, [open, cursor, base]);

  const shown = selectedIndex >= 0 ? items[selectedIndex] : null;

  return (
    <>
      <button
        {...a11y}
        ref={anchorRef}
        type="button"
        data-ui="select"
        data-size={size}
        data-open={open || undefined}
        data-disabled={disabled || undefined}
        data-invalid={a11y['aria-invalid'] ? 'true' : undefined}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={open && cursor >= 0 ? `${base}-${cursor}` : undefined}
        disabled={disabled}
        onClick={() => setOpen((was) => !was)}
        onKeyDown={onKeyDown}
        className={cx(frame.frame, frame.trigger, styles.trigger)}
      >
        <span className={cx(styles.value, !shown && styles.empty)} data-mono={mono || undefined}>
          {shown ? shown.label : (placeholder ?? '')}
        </span>
        <Icon src={ChevronDown} tone="muted" className={styles.arrow} />
      </button>

      <Popover
        open={open && !disabled}
        anchorRef={anchorRef}
        onClose={() => setOpen(false)}
        role="listbox"
        id={listboxId}
        matchAnchorWidth
        closeOnScroll={false}
      >
        {items.map((option, index) => (
          <div
            key={option.value === '' ? '__clear' : option.value}
            id={`${base}-${index}`}
            data-ui="option"
            role="option"
            aria-selected={index === selectedIndex}
            data-active={index === cursor || undefined}
            data-selected={index === selectedIndex || undefined}
            data-disabled={option.disabled || undefined}
            data-clear={option.value === '' && clearable ? true : undefined}
            className={cx(rows.item, styles.option)}
            onPointerEnter={() => {
              if (!option.disabled) setCursor(index);
            }}
            onClick={() => commit(index)}
          >
            <span className={styles.text}>{option.label}</span>
            {option.hint ? <span className={styles.hint}>{option.hint}</span> : null}
          </div>
        ))}
      </Popover>
    </>
  );
}
