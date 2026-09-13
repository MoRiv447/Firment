import { useEffect, useRef } from 'react';
import type { ReactNode, Ref } from 'react';

import { useFieldProps } from './Field';
import styles from './Checkbox.module.css';

/**
 * A checkbox, including the in-between state a "select all" needs.
 *
 * The native input is kept and painted over rather than replaced by a
 * `<button role="checkbox">`: space, the label click and the form value then come
 * for free, and reimplementing them is how a hand-rolled control ends up worse
 * than the one it replaced.
 *
 * `indeterminate` is a property on the node, not an attribute, so no JSX can set
 * it -- hence the effect below and the forwarded ref. It is drawn as a rule
 * instead of a tick so "some of these" cannot be misread as "none of these", and
 * the browser announces it as `mixed` on its own.
 */
export function Checkbox({
  checked,
  onChange,
  label,
  disabled = false,
  id,
  inputRef,
}: {
  checked: boolean | 'indeterminate';
  onChange: (next: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  id?: string;
  /** For the caller that also has to set `indeterminate` imperatively. */
  inputRef?: Ref<HTMLInputElement>;
}) {
  const a11y = useFieldProps({ id });
  const mixed = checked === 'indeterminate';
  const own = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (own.current) own.current.indeterminate = mixed;
  }, [mixed]);

  // Two refs, one node: React 19 has no ref-merging helper, and a checkbox that
  // silently dropped the caller's ref would be a trap worth avoiding.
  const attach = (node: HTMLInputElement | null) => {
    own.current = node;
    if (typeof inputRef === 'function') inputRef(node);
    else if (inputRef) inputRef.current = node;
  };

  return (
    <label
      data-ui="checkbox"
      data-on={checked !== false || undefined}
      data-mixed={mixed || undefined}
      className={styles.row}
    >
      <input
        {...a11y}
        id={a11y.id}
        ref={attach}
        type="checkbox"
        className={styles.input}
        checked={checked === true}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label ? <span className={styles.text}>{label}</span> : null}
    </label>
  );
}
