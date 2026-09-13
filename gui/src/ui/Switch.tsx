import type { Ref } from 'react';

import { useFieldProps } from './Field';
import styles from './Switch.module.css';

/**
 * A two-state switch that takes effect the moment you move it.
 *
 * `role="switch"` on a `<button>` rather than a checkbox, because that is what it
 * is here: every setting in this app writes through immediately (`save_settings`
 * on change), so there is no form to submit and no "apply" step to agree with. A
 * checkbox would promise a state that only becomes true when something else
 * happens.
 *
 * The pill is a child, not the button itself, so the label text can live INSIDE
 * the control: that gives the switch its accessible name for free, makes the
 * whole row clickable, and removes the `<label for>` that would not have worked
 * on a `<button>` anyway.
 */
export function Switch({
  checked,
  onChange,
  label,
  name,
  disabled = false,
  id,
  ref,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  /** Text inside the control. Omit it in a dense column and pass `name`. */
  label?: string;
  /** Accessible name for the label-less form. */
  name?: string;
  disabled?: boolean;
  id?: string;
  ref?: Ref<HTMLButtonElement>;
}) {
  const a11y = useFieldProps({ id });

  return (
    <button
      {...a11y}
      id={a11y.id}
      ref={ref}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label ? undefined : name}
      disabled={disabled}
      data-on={checked || undefined}
      data-ui="switch"
      className={styles.root}
      onClick={() => onChange(!checked)}
    >
      <span className={styles.pill} aria-hidden="true">
        <span className={styles.knob} />
      </span>
      {label ? <span className={styles.text}>{label}</span> : null}
    </button>
  );
}
