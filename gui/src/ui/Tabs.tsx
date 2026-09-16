import { useRef } from 'react';
import { Tooltip, useTooltip } from './Tooltip';
import type { KeyboardEvent, ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';

import { Icon } from './Icon';
import styles from './Tabs.module.css';

export interface TabItem {
  key: string;
  label: ReactNode;
  icon?: LucideIcon;
  /** A count or a state dot: "3 todos", "running". Not a replacement for the label. */
  meta?: ReactNode;
  disabled?: boolean;
}

/**
 * The tab strip that switches a pane's content.
 *
 * It renders the strip only. Which panel is mounted is the pane's business, and a
 * `Tabs` that also owned the content would force every pane in the inspector to
 * keep its state inside this file -- which is how `WorkbenchView.tsx` ended up 1530
 * lines.
 *
 * The active tab is marked with a rule rather than a filled background: a strip of
 * four tabs where one is painted like a selected menu item reads as a list, and the
 * rule says "this is the column you are looking at" the way a table header does.
 */
export function Tabs({
  items,
  active,
  onChange,
  ariaLabel,
  idPrefix,
}: {
  items: TabItem[];
  active: string;
  onChange: (key: string) => void;
  ariaLabel: string;
  /**
   * Lets the tab carry `aria-controls` and the panel carry the matching id, which
   * is what a screen reader uses to jump from the strip to the content.
   */
  idPrefix?: string;
}) {
  const nodes = useRef<Array<HTMLButtonElement | null>>([]);

  const step = (event: KeyboardEvent, delta: number) => {
    const at = items.findIndex((item) => item.key === active);
    if (at < 0) return;
    event.preventDefault();
    // Skip the disabled ones rather than landing on a tab that cannot answer.
    for (let offset = 1; offset <= items.length; offset += 1) {
      const raw = at + delta * offset;
      const next = ((raw % items.length) + items.length) % items.length;
      const item = items[next];
      if (item && !item.disabled) {
        onChange(item.key);
        nodes.current[next]?.focus({ preventScroll: true });
        return;
      }
    }
  };

  return (
    <div
      data-ui="tabs"
      role="tablist"
      aria-label={ariaLabel}
      className={styles.strip}
      onKeyDown={(event) => {
        if (event.key === 'ArrowRight') step(event, 1);
        else if (event.key === 'ArrowLeft') step(event, -1);
      }}
    >
      {items.map((item, index) => (
        <TabButton
          key={item.key}
          item={item}
          selected={item.key === active}
          id={idPrefix ? `${idPrefix}-tab-${item.key}` : undefined}
          controls={idPrefix ? `${idPrefix}-panel-${item.key}` : undefined}
          register={(node) => {
            nodes.current[index] = node;
          }}
          onSelect={() => onChange(item.key)}
        />
      ))}
    </div>
  );
}

/**
 * One tab.
 *
 * A tab that has an icon renders **only** the icon, and its name lives in a
 * tooltip and in `aria-label`. Four labels do not fit a 210px inspector, and the
 * alternatives are worse than this: truncated to `Cha… Subag… To… Hard…` they are
 * unreadable, and left to overflow the strip grows a scrollbar that is louder than
 * the tabs are.
 *
 * `aria-label` is the part that is not cosmetic. An icon-only control with no name
 * does not exist for a screen reader, and the tooltip is not announced by default.
 */
function TabButton({
  item,
  selected,
  id,
  controls,
  register,
  onSelect,
}: {
  item: TabItem;
  selected: boolean;
  id?: string;
  controls?: string;
  register: (node: HTMLButtonElement | null) => void;
  onSelect: () => void;
}) {
  const tip = useTooltip<HTMLButtonElement>();
  const named = item.icon !== undefined;

  return (
    <>
      <button
        ref={(node) => {
          register(node);
          tip.anchorRef.current = node;
        }}
        id={id}
        type="button"
        role="tab"
        data-ui="tab"
        data-active={selected || undefined}
        data-icon-only={named || undefined}
        aria-selected={selected}
        aria-controls={controls}
        aria-label={typeof item.label === 'string' ? item.label : undefined}
        disabled={item.disabled}
        tabIndex={selected ? 0 : -1}
        className={styles.tab}
        onClick={onSelect}
        {...tip.triggerProps}
      >
        {item.icon ? <Icon src={item.icon} /> : <span className={styles.label}>{item.label}</span>}
        {item.meta ? <span className={styles.meta}>{item.meta}</span> : null}
      </button>
      {named && typeof item.label === 'string' ? <Tooltip tip={tip} text={item.label} /> : null}
    </>
  );
}
