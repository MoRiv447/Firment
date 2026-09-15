import { useEffect, useRef, useState } from 'react';

import { TextInput } from './Input';
import type { TextInputProps } from './Input';

export interface NumberFieldProps
  extends Omit<TextInputProps, 'value' | 'onChange' | 'type' | 'inputMode'> {
  value: number | undefined;
  /** Receives the parsed number, or `undefined` when the field is empty. */
  onValueChange: (next: number | undefined) => void;
  min?: number;
  max?: number;
}

/** What a typed string means as a number, or `undefined` if it means nothing. */
function parse(raw: string): number | undefined {
  const trimmed = raw.trim();
  if (trimmed === '') return undefined;
  const n = Number(trimmed);
  // `Number('')` is 0 and `Number(' ')` is 0, which is why the empty case is
  // handled first; `Number('12abc')` is NaN and `Number('1e9')` is fine.
  return Number.isFinite(n) ? n : undefined;
}

/**
 * The numeric field.
 *
 * This exists so that one place decides what a number field means, rather than
 * three views each deciding separately. The five call sites need exactly two
 * rules, and they are the two that drift when they are copied:
 *
 *   * **out of range is clamped, not rejected.** Typing past `max` gives the
 *     caller `max`, and the text normalises on blur. A field that reported an
 *     out-of-range number would push the decision into every caller; one that
 *     refused to report anything would leave the caller's state stale while the
 *     box showed something else.
 *   * **empty is `undefined`, not `0`.** "No limit set" and "the limit is zero"
 *     are different settings, and `Number('')` is 0.
 *
 * The text is kept locally because a number is not what is being typed: `1.`,
 * `-` and `0.` are all legitimate halfway states that parse to nothing, and a
 * field that reported on every keystroke from the *value* would delete them as
 * they were typed. The text is not resynced from `value` while the caller is
 * reacting to our own report -- a field that rewrote `999` to `100` mid-keystroke
 * would be unusable -- only when the value changes from outside.
 *
 * No `step`: there is no stepper here, and a prop that promised one would be a
 * lie. `suffix` carries the unit, which is what the call sites were writing next
 * to the box by hand.
 */
export function NumberField({
  value,
  onValueChange,
  min,
  max,
  suffix,
  disabled,
  ...rest
}: NumberFieldProps) {
  const toText = (n: number | undefined) => (n === undefined ? '' : String(n));
  const [text, setText] = useState(() => toText(value));
  const lastReported = useRef<number | undefined>(value);

  useEffect(() => {
    // A value that did not come from this field: the caller reset it, or loaded
    // something else. Anything else is the caller echoing back what we reported,
    // and rewriting the box then would fight the typist.
    if (value !== lastReported.current) {
      lastReported.current = value;
      setText(toText(value));
    }
  }, [value]);

  const clamp = (n: number) => {
    if (min !== undefined && n < min) return min;
    if (max !== undefined && n > max) return max;
    return n;
  };

  const handle = (raw: string) => {
    setText(raw);
    const parsed = parse(raw);
    const next = parsed === undefined ? undefined : clamp(parsed);
    lastReported.current = next;
    onValueChange(next);
  };

  return (
    <TextInput
      {...rest}
      suffix={suffix}
      disabled={disabled}
      value={text}
      inputMode="numeric"
      onChange={(e) => handle(e.target.value)}
      onBlur={() => setText(toText(lastReported.current))}
    />
  );
}
