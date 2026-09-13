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
  id?: string;
  ariaLabel?: string;
}) {
  const base = useId();
  const listboxId = `${base}-listbox`;
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(-1);

  const a11y = useFieldProps({ id, invalid });

  const selectedIndex = useMemo(
    () => options.findIndex((option) => option.value === value),
    [options, value],
  );

  // A disabled row keeps an empty label rather than its own: typeahead matches by
  // prefix, so "" is a row the keyboard cannot land on -- which is the same
  // answer the pointer gets, where the row is painted dead and Enter refuses it.
  const labels = useMemo(
    () =>
      options.map((option) =>
        option.disabled ? '' : typeof option.label === 'string' ? option.label : '',
      ),
    [options],
  );

  // Start the cursor on the current value, not on the top of the list: a user who
  // opens a picker and presses Enter should not have just changed the setting.
  useEffect(() => {
    if (open) setCursor(selectedIndex);
  }, [open, selectedIndex]);

  const commit = (index: number) => {
    const option = options[index];
    if (!option || option.disabled) return;
    onChange(option.value);
    setOpen(false);
  };

  const listKeys = useListKeyboard({
    count: options.length,
    active: cursor,
    onActive: setCursor,
    onCommit: commit,
    onClose: () => setOpen(false),
    labels,
    isEnabled: (index) => !options[index]?.disabled,
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

  const shown = selectedIndex >= 0 ? options[selectedIndex] : null;

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
        {options.map((option, index) => (
          <div
            key={option.value}
            id={`${base}-${index}`}
            data-ui="option"
            role="option"
            aria-selected={index === selectedIndex}
            data-active={index === cursor || undefined}
            data-selected={index === selectedIndex || undefined}
            data-disabled={option.disabled || undefined}
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
