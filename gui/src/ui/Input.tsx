import { Search } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { InputHTMLAttributes, Ref, TextareaHTMLAttributes } from 'react';

import { cx } from './cx';
import frame from './control.module.css';
import { Icon } from './Icon';
import { useFieldProps } from './Field';
import styles from './Input.module.css';

/** The native `size` attribute is a width in characters; here it is a height. */
type NativeInput = Omit<InputHTMLAttributes<HTMLInputElement>, 'size'>;
type NativeTextArea = Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, 'rows'>;

export interface TextInputProps extends NativeInput {
  size?: 'sm' | 'md' | 'lg';
  /** Monospace: a path, a port name, a hex value, a commit sha. */
  mono?: boolean;
  invalid?: boolean;
  icon?: LucideIcon;
  /** A unit after the text -- `KB`, `ms`, `baud`. Reads as part of the field. */
  suffix?: string;
  ref?: Ref<HTMLInputElement>;
}

/**
 * The text field.
 *
 * The box it sits in is `control.module.css`, the same frame `Select` and
 * `TextArea` wear, so a panel of mixed controls lines up without anyone
 * measuring it.
 */
export function TextInput({
  size = 'md',
  mono = false,
  invalid = false,
  icon: Glyph,
  suffix,
  id,
  disabled,
  ref,
  ...rest
}: TextInputProps) {
  const a11y = useFieldProps({ id, invalid });

  return (
    <span
      className={frame.frame}
      data-size={size}
      data-mono={mono || undefined}
      data-disabled={disabled || undefined}
      data-invalid={a11y['aria-invalid'] ? 'true' : undefined}
    >
      {Glyph ? <Icon src={Glyph} tone="muted" className={styles.glyph} /> : null}
      <input
        {...rest}
        {...a11y}
        ref={ref}
        disabled={disabled}
        data-ui="input"
        className={styles.field}
      />
      {suffix ? <span className={styles.suffix}>{suffix}</span> : null}
    </span>
  );
}

export interface TextAreaProps extends NativeTextArea {
  /** A line count, not a pixel height -- see the note on `TextArea`. */
  rows?: number;
  mono?: boolean;
  invalid?: boolean;
  ref?: Ref<HTMLTextAreaElement>;
}

/**
 * A growing box for the things people paste into it.
 *
 * `rows` is a count of lines rather than a height so the box follows the type step
 * -- eight rows at `--fs-body` is a different box than eight rows at
 * `--fs-minor`, and "eight lines" is what the author meant either way.
 */
export function TextArea({
  mono = false,
  invalid = false,
  id,
  disabled,
  rows = 4,
  ref,
  ...rest
}: TextAreaProps) {
  const a11y = useFieldProps({ id, invalid });

  return (
    <span
      className={frame.frame}
      data-multiline="true"
      data-mono={mono || undefined}
      data-disabled={disabled || undefined}
      data-invalid={a11y['aria-invalid'] ? 'true' : undefined}
    >
      <textarea
        {...rest}
        {...a11y}
        ref={ref}
        disabled={disabled}
        rows={rows}
        data-ui="textarea"
        className={cx(styles.field, styles.textarea)}
      />
    </span>
  );
}

/** A search box: the field with the glyph that says what it filters. */
export function SearchInput(props: Omit<TextInputProps, 'icon' | 'type'>) {
  return <TextInput {...props} type="search" icon={Search} />;
}
