import { useId } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { PanelRightClose, PanelRightOpen } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { Icon, IconButton, Tabs, Tooltip, useTooltip } from '../ui';
import { Splitter } from './Splitter';
import styles from './Inspector.module.css';

/**
 * The column's floor and ceiling, in px.
 *
 * 240 is where a diff starts wrapping; 560 is where the transcript on the left
 * stops being readable at the window's minimum width. Both numbers are the
 * column's business, not the caller's, so they live here.
 */
const MIN_WIDTH = 240;
const MAX_WIDTH = 560;

export interface InspectorTab {
  key: string;
  label: string;
  /** Shown in place of the label when the column is collapsed to the rail. */
  icon?: LucideIcon;
  /** A count worth showing next to the label. Omitted when there is nothing. */
  badge?: number;
  /**
   * The pane scrolls itself and takes the whole body box. For a log that has to
   * keep its own header in place; prose panes leave it off.
   */
  fill?: boolean;
  content: ReactNode;
}

/** One entry in the collapsed rail, with its name in a tooltip. */
function RailButton({
  label,
  icon: Glyph,
  onClick,
}: {
  label: string;
  icon: LucideIcon;
  onClick: () => void;
}) {
  const tip = useTooltip<HTMLButtonElement>();
  return (
    <>
      <IconButton
        ref={tip.anchorRef}
        {...tip.triggerProps}
        size="sm"
        label={label}
        icon={Glyph}
        onClick={onClick}
      />
      {/* Left: the rail is against the window's right edge. */}
      <Tooltip tip={tip} text={label} side="left" />
    </>
  );
}

/**
 * The right-hand column: what the session has produced, pinned next to it.
 *
 * It exists because the alternative -- what the app did until now -- was to give
 * each of these its own top-level tab. A tab is a place you go; these are things
 * you glance at *while* reading the conversation, and going to a tab means leaving
 * the conversation behind.
 *
 * The strip is `Tabs`, which means it answers to the arrow keys and announces
 * `aria-selected`; it used to be four hand-rolled `<button>`s with no `role` at
 * all, so a screen reader read them as four buttons in a row and never said which
 * one you were looking at.
 *
 * Two ideas about width, and they are separate: the column clamps between
 * `MIN_WIDTH` and `MAX_WIDTH`, and the drag handle in `Splitter` decides where in
 * that range it sits. The clamp is what makes the handle safe to leave at
 * full-auto.
 */
export function Inspector({
  tabs,
  active,
  onActiveChange,
  open,
  onToggle,
  width,
  onResize,
}: {
  tabs: InspectorTab[];
  /**
   * Which tab is up, held by the caller.
   *
   * It used to be this component's own `useState`, and that is why nothing else in
   * the window could send you here: a tool card that wants to show its diff in the
   * Changes pane has to be able to say "Changes", and a private piece of state is
   * not a thing a sibling can reach.
   */
  active: string;
  onActiveChange: (key: string) => void;
  open: boolean;
  onToggle: () => void;
  width: number;
  onResize: (next: number) => void;
}) {
  const prefix = useId();
  const current = tabs.find((tab) => tab.key === active) ?? tabs[0];

  // The pane a rail button opens is the one you wanted, so the act of collapsing
  // must not forget which that was.
  const openTab = (key: string) => {
    onActiveChange(key);
    if (!open) onToggle();
  };

  const body = (
    <div
      data-ui="inspector-body"
      data-fill={current?.fill ? 'true' : undefined}
      className={styles.body}
    >
      {/*
        Every pane is mounted and the inactive ones are hidden, because rendering only
        `current.content` — which is what this did — unmounts a pane the moment you look away.
        For the Hardware pane that is not a cosmetic loss: it holds the serial monitor and the
        flash view, their `busy` flags and results, and the `onHardwareExit` listener that tells
        the app the monitor closed. Leave Hardware to read a diff and come back to a port nobody
        is watching.
  
        Each wrapper carries the panel role and the id `Tabs` points `aria-controls` at, so the
        three tabs' controls all resolve — they used to name an element that existed only while
        that tab was up.
      */}
      {tabs.map((tab) => (
        <div
          key={tab.key}
          hidden={tab.key !== current?.key}
          className={tab.fill ? styles.fillPane : styles.pane}
          id={`${prefix}-panel-${tab.key}`}
          role="tabpanel"
          aria-labelledby={`${prefix}-tab-${tab.key}`}
        >
          {tab.content}
        </div>
      ))}
    </div>
  );

  if (!open) {
    return (
      <>
        <aside data-ui="inspector-rail" className={styles.rail}>
          <RailButton label="Open the inspector" icon={PanelRightOpen} onClick={onToggle} />
          {tabs.map((tab) => (
            <RailButton
              key={tab.key}
              label={tab.badge ? `${tab.label}, ${tab.badge}` : tab.label}
              icon={tab.icon ?? PanelRightOpen}
              onClick={() => openTab(tab.key)}
            />
          ))}
        </aside>
        {/*
          Collapsed is not closed. This used to return the rail alone, which unmounted every pane
          and threw away the same state the comment above is about — `8448fb3` fixed the switch
          inside the pane, `fe20f00` the switch between panes, and collapsing was the third way to
          do it. Collapse the inspector while a flash is running and the completion was never
          recorded; reopening said "No run yet". A hidden container keeps the subtrees mounted and
          out of the layout and the accessibility tree at the same time.
        */}
        <div data-ui="inspector-parked" hidden>
          {body}
        </div>
      </>
    );
  }

  return (
    <>
      <Splitter value={width} min={MIN_WIDTH} max={MAX_WIDTH} onResize={onResize} />
      <aside
        data-ui="inspector"
        style={{ '--inspector-w': `${width}px` } as CSSProperties}
        className={styles.column}
      >
        <div className={styles.head}>
          <div className={styles.strip}>
            <Tabs
              ariaLabel="Inspector"
              idPrefix={prefix}
              active={current?.key ?? ''}
              onChange={onActiveChange}
              items={tabs.map((tab) => ({
                key: tab.key,
                label: tab.label,
                icon: tab.icon,
                meta: tab.badge ? tab.badge : undefined,
              }))}
            />
          </div>
          <CollapseToggle onToggle={onToggle} />
        </div>
        {body}
      </aside>
    </>
  );
}

/** The head's last control, on the strip's own line. */
function CollapseToggle({ onToggle }: { onToggle: () => void }) {
  const tip = useTooltip<HTMLButtonElement>();
  const label = 'Collapse the inspector';
  return (
    <>
      <button
        {...tip.triggerProps}
        ref={tip.anchorRef}
        type="button"
        aria-label={label}
        onClick={onToggle}
        className={styles.toggle}
      >
        <Icon src={PanelRightClose} />
      </button>
      <Tooltip tip={tip} text={label} side="left" />
    </>
  );
}
