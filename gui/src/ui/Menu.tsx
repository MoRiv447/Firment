import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { ReactNode, RefObject } from 'react';
import type { LucideIcon } from 'lucide-react';

import { cx } from './cx';
import floating from './floating.module.css';
import { Icon } from './Icon';
import styles from './Menu.module.css';
import { overlayRoot } from './portal';
import type { Align, Side } from './types';
import { useListKeyboard } from './useListKeyboard';
import { usePopover } from './usePopover';

export interface MenuItem {
  key: string;
  label: ReactNode;
  icon?: LucideIcon;
  /**
   * "This one destroys something." Painted with the removed-diff ink, which is the
   * same colour a failed step and a deleted line use, so the warning is one
   * vocabulary rather than three.
   */
  danger?: boolean;
  disabled?: boolean;
  /** A shortcut or a count, on the right. */
  hint?: string;
  onSelect?: () => void;
}

export interface MenuSeparator {
  separator: true;
  key: string;
}

export type MenuEntry = MenuItem | MenuSeparator;

const isSeparator = (entry: MenuEntry): entry is MenuSeparator => 'separator' in entry;
const labelOf = (entry: MenuEntry): string =>
  isSeparator(entry) ? '' : typeof entry.label === 'string' ? entry.label : '';

/**
 * A dropdown of commands, opened from a button.
 *
 * Built on `usePopover` rather than on `<Popover>`, because the panel element here
 * is the menu: it carries `role="menu"`, the `aria-activedescendant` and the
 * keydown listener, and forwarding all three through a wrapper would be a list of
 * escape hatches pretending to be an API.
 *
 * The keyboard model is the one ARIA asks for: focus moves into the menu, the rows
 * are `tabIndex={-1}`, and `aria-activedescendant` names the row the arrows are
 * on. Two details that are invisible until they are missing:
 *
 * * Pointer and keyboard share one highlight. Hovering moves the cursor, so
 *   reaching for the mouse mid-list does not leave two rows looking chosen.
 * * Escape, Enter and a click all hand focus back to the trigger. A menu that
 *   closes onto `document.body` sends the next Tab to the top of the document.
 */
export function Menu({
  open,
  anchorRef,
  onClose,
  items,
  side = 'bottom',
  align = 'end',
  labelledBy,
}: {
  open: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  items: MenuEntry[];
  side?: Side;
  align?: Align;
  /** The trigger's id, so the menu announces itself as belonging to it. */
  labelledBy?: string;
}) {
  const base = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [cursor, setCursor] = useState(0);

  const { style, side: placed, ready } = usePopover({
    open,
    anchorRef,
    panelRef,
    side,
    align,
    onDismiss: onClose,
  });

  /** Item indexes the arrows can land on: no separators, no dead items. */
  const positions = useMemo(() => {
    const map = new Map<number, number>();
    items.forEach((entry, index) => {
      if (!isSeparator(entry) && !entry.disabled) map.set(index, map.size);
    });
    return map;
  }, [items]);

  const labels = useMemo(() => {
    const out: string[] = [];
    items.forEach((entry, index) => {
      if (positions.has(index)) out.push(labelOf(entry));
    });
    return out;
  }, [items, positions]);

  // Re-opening must not inherit a cursor from the last time it was open: the
  // highlight would name a row the user never chose, and Enter would run it.
  useEffect(() => {
    if (!open) return;
    setCursor(0);
    panelRef.current?.focus({ preventScroll: true });
  }, [open]);

  const closeAndRestore = () => {
    onClose();
    anchorRef.current?.focus({ preventScroll: true });
  };

  const indexOfCursor = () => {
    for (const [itemIndex, position] of positions) if (position === cursor) return itemIndex;
    return undefined;
  };

  // A model list longer than the panel is scrolled, not paginated: an arrow key
  // that moves a cursor nobody can see is a control that appears broken.
  useEffect(() => {
    if (!open) return;
    const active = panelRef.current?.querySelector('[data-active="true"]');
    active?.scrollIntoView?.({ block: 'nearest' });
  }, [open, cursor]);

  const commit = (index: number | undefined) => {
    if (index === undefined) return;
    const entry = items[index];
    if (!entry || isSeparator(entry)) return;
    // Close first, then run: an `onSelect` that navigates or opens a confirm
    // should not have to do it underneath a menu that is still up.
    closeAndRestore();
    entry.onSelect?.();
  };

  const onKeyDown = useListKeyboard({
    count: positions.size,
    active: cursor,
    onActive: setCursor,
    onCommit: () => commit(indexOfCursor()),
    onClose: closeAndRestore,
    labels,
  });

  if (!open) return null;

  const activeIndex = indexOfCursor();

  return createPortal(
    <div
      ref={panelRef}
      data-ui="menu"
      data-side={placed}
      data-ready={ready || undefined}
      role="menu"
      aria-labelledby={labelledBy}
      aria-activedescendant={activeIndex === undefined ? undefined : `${base}-${activeIndex}`}
      tabIndex={-1}
      style={style}
      onKeyDown={(event) => onKeyDown(event)}
      className={floating.panel}
    >
      {items.map((entry, index) =>
        isSeparator(entry) ? (
          <div key={entry.key} data-ui="menu-separator" role="separator" className={styles.sep} />
        ) : (
          <button
            key={entry.key}
            id={`${base}-${index}`}
            type="button"
            role="menuitem"
            data-ui="menu-item"
            data-active={positions.get(index) === cursor || undefined}
            data-danger={entry.danger || undefined}
            disabled={entry.disabled}
            data-disabled={entry.disabled || undefined}
            tabIndex={-1}
            className={cx(floating.item, styles.item)}
            onPointerEnter={() => {
              const position = positions.get(index);
              if (position !== undefined) setCursor(position);
            }}
            onClick={() => commit(index)}
          >
            {entry.icon ? <Icon src={entry.icon} tone="muted" /> : null}
            <span className={styles.text}>{entry.label}</span>
            {entry.hint ? <span className={styles.hint}>{entry.hint}</span> : null}
          </button>
        ),
      )}
    </div>,
    overlayRoot(),
  );
}
