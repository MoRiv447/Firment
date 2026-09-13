import type { CSSProperties, Ref } from 'react';

import { useFieldProps } from './Field';
import styles from './Slider.module.css';

/**
 * A range control with its value written next to it.
 *
 * No frame around it. A bordered box is there to say "type in here", and a slider
 * inside one reads as a text field that happens to have a line through it -- so
 * this is the track, the thumb, and a readout, and it lines up with the other
 * controls by sharing their height rather than their border.
 *
 * The readout is not decoration: a slider that only says "somewhere in the middle"
 * cannot be set to a value you already know, and half the numbers here (a baud
 * rate, a token budget) are things people want to type. Callers that need the
 * typed form use `TextInput` with `suffix` instead, and both write the same value.
 */
export function Slider({
  value,
  onChange,
  min = 0,
  max = 100,
  step = 1,
  disabled = false,
  id,
  format,
  name,
  ref,
}: {
  value: number;
  onChange: (next: number) => void;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  id?: string;
  /** What the readout says. Numbers without a unit are ambiguous, so pass one. */
  format?: (value: number) => string;
  name?: string;
  ref?: Ref<HTMLInputElement>;
}) {
  const a11y = useFieldProps({ id });
  const span = max - min;
  const clamped = span === 0 ? 0 : (Math.min(max, Math.max(min, value)) - min) / span;

  return (
    <span className={styles.row} data-disabled={disabled || undefined}>
      <input
        {...a11y}
        id={a11y.id}
        ref={ref}
        type="range"
        className={styles.input}
        data-ui="slider"
        name={name}
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        /* The filled half of the track is geometry the browser already knows, so
           it is handed over as a token-shaped custom property rather than written
           into an inline style. */
        style={{ '--slider-fill': `${(clamped * 100).toFixed(2)}%` } as CSSProperties}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      {format ? <span className={styles.readout}>{format(value)}</span> : null}
    </span>
  );
}
