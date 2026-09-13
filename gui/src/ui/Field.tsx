import { createContext, useContext, useId, useMemo } from 'react';
import type { ReactNode } from 'react';

import { cx } from './cx';
import styles from './Field.module.css';

/**
 * What a control needs in order to be properly labelled.
 *
 * Not obvious from the props, which is why it is passed down rather than typed in
 * at each call site: an unlabelled input is a form that only sighted mouse users
 * can fill in, and `aria-describedby` is the difference between "Port" and "Port
 * -- the serial device the flasher will use".
 */
export interface FieldInfo {
  controlId: string;
  describedBy?: string;
  invalid: boolean;
}

const FieldContext = createContext<FieldInfo | null>(null);

export function useField(): FieldInfo | null {
  return useContext(FieldContext);
}

/**
 * The `id` / `aria-describedby` / `aria-invalid` a control should carry, merged
 * with whatever the caller already set.
 *
 * Every control in this layer calls it, so a `Field` wrapping a `Select` works the
 * same as a `Field` wrapping a `TextInput` -- with antd's `Form` that wiring was
 * the library's job, and "we removed the library" is exactly when it goes missing.
 */
export function useFieldProps(props: { id?: string; invalid?: boolean } = {}): {
  id?: string;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean;
} {
  const field = useField();
  const invalid = props.invalid || !!field?.invalid;
  if (!field) return { id: props.id, 'aria-invalid': invalid || undefined };
  return {
    id: props.id ?? field.controlId,
    'aria-describedby': field.describedBy,
    'aria-invalid': invalid || undefined,
  };
}

/**
 * A label, a control, and the two lines of prose that explain it.
 *
 * `SettingsView.tsx` has ~40 settings, each of which was assembled by hand out of
 * a `<Text>` and a `<div>`, so the same relationship -- label above, hint below,
 * error instead of hint when there is one -- was written forty times and differed
 * in forty places. The hierarchy here is deliberate: an error replaces the hint
 * rather than stacking under it, because a hint under a red line is text nobody
 * reads while something is wrong.
 *
 * `inline` is the dense form: label in a column, control beside it. The serial and
 * hardware settings panels need it (a form of 12 short fields read as a wall at
 * full width), and it is a variant rather than a second component so the two can
 * never disagree about what the label means.
 */
export function Field({
  label,
  hint,
  error,
  required = false,
  inline = false,
  as = 'div',
  children,
  className,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  inline?: boolean;
  /** `fieldset` when the group is a real grouping (a settings section). */
  as?: 'div' | 'fieldset';
  children: ReactNode;
  className?: string;
}) {
  const uid = useId();
  const controlId = `${uid}-control`;
  const hintId = `${uid}-hint`;
  const errorId = `${uid}-error`;

  const value = useMemo<FieldInfo>(
    () => ({
      controlId,
      describedBy: (error ? errorId : hint ? hintId : undefined) || undefined,
      invalid: !!error,
    }),
    [controlId, hintId, errorId, !!error, !!hint],
  );

  const isGroup = as === 'fieldset';
  const Root = isGroup ? 'fieldset' : 'div';
  const Label = isGroup ? 'legend' : 'label';

  return (
    <Root
      data-ui="field"
      data-inline={inline || undefined}
      data-invalid={error ? 'true' : undefined}
      className={cx(styles.field, className)}
    >
      {/* `className` is allowed on Field and not on most of this layer because
          Field is a container: whether it spans one column of a settings grid or
          two is the grid's question, not the label's. */}
      <Label className={styles.label} htmlFor={isGroup ? undefined : controlId}>
        {label}
        {required ? (
          <span className={styles.required} aria-hidden="true">
            *
          </span>
        ) : null}
      </Label>
      <FieldContext.Provider value={value}>
        <div className={styles.control}>{children}</div>
      </FieldContext.Provider>
      {error ? (
        <p id={errorId} role="alert" className={cx(styles.note, styles.error)}>
          {error}
        </p>
      ) : hint ? (
        <p id={hintId} className={cx(styles.note, styles.hint)}>
          {hint}
        </p>
      ) : null}
    </Root>
  );
}
