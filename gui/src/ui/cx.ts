/**
 * Join the class names a component actually wants, dropping the ones it does not.
 *
 * CSS Modules hash every class, so a component cannot match on a class name and
 * state is carried by `data-*` attributes instead. What is left for `cx` is the
 * structural case: root plus a conditional modifier from the caller's side.
 */
export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}
