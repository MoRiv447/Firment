import { createContext, useContext, useId, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';

import styles from './Radio.module.css';

interface RadioGroupInfo {
  name: string;
  value: string;
  onSelect: (next: string) => void;
  disabled: boolean;
}

const RadioContext = createContext<RadioGroupInfo | null>(null);

/**
 * A set of mutually exclusive choices.
 *
 * `name` is generated rather than required. Two radios that share a visual group
 * but not a `name` still look like one choice to a user and behave as two to the
 * browser -- arrow keys will not move between them, and both can be selected
 * after a form reset. Deriving it from `useId()` removes the mistake instead of
 * documenting it away.
 *
 * There is no `role="radiogroup"` here: the group label belongs to the
 * `<Field as="fieldset">` that wraps it, and a second ARIA group around a real
 * set of native radios would just give a screen reader two things to announce.
 */
export function RadioGroup({
  value,
  onChange,
  children,
  name,
  disabled = false,
  layout = 'stack',
}: {
  value: string;
  onChange: (next: string) => void;
  children: ReactNode;
  name?: string;
  disabled?: boolean;
  /** `stack` for a settings list, `row` for two or three short options. */
  layout?: 'stack' | 'row';
}) {
  // useId is stable across renders; a counter would give a different name after
  // every re-render and split the group in the browser's own bookkeeping.
  const uid = useId();
  // The caller's `onChange` is an inline arrow at every call site. Keeping it in a
  // ref means the context value only changes when the choice does, so typing in a
  // neighbouring field does not re-render the whole group.
  const latest = useRef(onChange);
  latest.current = onChange;

  const info: RadioGroupInfo = useMemo(
    () => ({ name: name ?? uid, value, onSelect: (next: string) => latest.current(next), disabled }),
    [name, uid, value, disabled],
  );

  return (
    <div data-ui="radio-group" data-layout={layout} className={styles.group}>
      <RadioContext.Provider value={info}>{children}</RadioContext.Provider>
    </div>
  );
}

/**
 * One choice, with an optional second line.
 *
 * The second line is the reason this exists: half the radios in the settings
 * screen are a name plus a sentence about what picking it costs, and antd's
 * `Radio` had nowhere to put that, so the prose ended up as a sibling `<div>` with
 * its own padding -- which is how an explanatory line ends up 3px away from its
 * own control and next to the one below it.
 */
export function Radio({
  value,
  label,
  hint,
  disabled = false,
  onChange,
}: {
  value: string;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
  /** Only needed when the radio is outside a `RadioGroup`. */
  onChange?: (next: string) => void;
}) {
  const group = useContext(RadioContext);
  const checked = group ? group.value === value : false;
  const isDisabled = disabled || !!group?.disabled;

  return (
    <label
      data-ui="radio"
      data-on={checked || undefined}
      data-disabled={isDisabled || undefined}
      className={styles.row}
    >
      <input
        className={styles.input}
        type="radio"
        name={group?.name}
        value={value}
        checked={checked}
        disabled={isDisabled}
        onChange={() => (group ? group.onSelect(value) : onChange?.(value))}
      />
      <span className={styles.stack}>
        <span className={styles.text}>{label}</span>
        {hint ? <span className={styles.hint}>{hint}</span> : null}
      </span>
    </label>
  );
}
