import { useRef } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';

import { Icon } from './Icon';
import styles from './Segmented.module.css';
import type { Size } from './types';

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
  icon?: LucideIcon;
  /** The untruncated text, for a label that has to fit a narrow pane. */
  title?: string;
  disabled?: boolean;
}

/**
 * A row of mutually exclusive choices, all of them visible at once.
 *
 * `Select` is for a list too long to scan; this is for the two, three or four
 * options where seeing all of them is the point. The old tree used antd's
 * `Segmented`, which renders `<label>`s around hidden radio inputs and positions a
 * thumb of its own; this is a `radiogroup` of buttons, which is the same semantics
 * with one fewer mechanism and no thumb to keep in sync with the type scale.
 *
 * Arrow keys move the selection *and* commit it, which is what a radio group does.
 * Roving `tabIndex` makes the group one Tab stop rather than four.
 */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  size = 'md',
  disabled = false,
  ariaLabel,
}: {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /** `sm` is for a control inside a 40px row; `lg` is for an empty screen. */
  size?: Size;
  disabled?: boolean;
  /** Required in practice: an unnamed radiogroup is announced as loose buttons. */
  ariaLabel?: string;
}) {
  const nodes = useRef<Array<HTMLButtonElement | null>>([]);

  const goTo = (index: number) => {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    nodes.current[index]?.focus({ preventScroll: true });
  };

  const step = (delta: number) => {
    const at = options.findIndex((option) => option.value === value);
    if (at < 0) {
      goTo(delta > 0 ? 0 : options.length - 1);
      return;
    }
    // Wrap rather than stop: with three options, either end should be two
    // keypresses away, and a dead arrow at the boundary reads as a stuck control.
    goTo((at + delta + options.length) % options.length);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (disabled) return;
    const { key } = event;
    if (key === 'ArrowRight' || key === 'ArrowDown') {
      event.preventDefault();
      step(1);
      return;
    }
    if (key === 'ArrowLeft' || key === 'ArrowUp') {
      event.preventDefault();
      step(-1);
      return;
    }
    if (key === 'Home') {
      event.preventDefault();
      goTo(0);
      return;
    }
    if (key === 'End') {
      event.preventDefault();
      goTo(options.length - 1);
    }
  };

  return (
    <div
      data-ui="segmented"
      data-size={size}
      data-disabled={disabled || undefined}
      role="radiogroup"
      aria-label={ariaLabel}
      aria-disabled={disabled || undefined}
      onKeyDown={onKeyDown}
      className={styles.group}
    >
      {options.map((option, index) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            ref={(node) => {
              nodes.current[index] = node;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled || option.disabled === true}
            tabIndex={selected ? 0 : -1}
            data-active={selected || undefined}
            className={styles.segment}
            title={option.title}
            onClick={() => onChange(option.value)}
          >
            {option.icon ? <Icon src={option.icon} /> : null}
            <span className={styles.label}>{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
