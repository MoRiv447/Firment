import { useId, useRef, useState } from 'react';
import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';
import { ChevronDown } from 'lucide-react';

import { Icon, Menu, StatusDot } from '../ui';
import type { ChipStatus, MenuItem } from '../ui';
import styles from './StatusBar.module.css';

/**
 * The bottom strip: what is happening right now, and the three readings you can
 * change from here.
 *
 * Every value here used to be a coloured dropdown chip in the header, competing
 * with the navigation for attention. They are readings, so they read as one quiet
 * line: a state mark per item, monospace for the values, and colour reserved for
 * the mark.
 *
 * `Reading` is the only shape a status bar entry takes, and the two ways it ends up
 * on screen are `StatusItem` (reports) and `StatusMenu` (reports, and opens a menu
 * of the values it can be set to). Keeping it to one shape is what stops this from
 * becoming another row of coloured pills -- the failure mode of the header it
 * replaces, which ended up with five hues in 200px because each chip chose its own
 * antd preset.
 */
interface ReadingProps {
  /** Drives the mark's colour. `neutral` is "no judgement". */
  kind: ChipStatus;
  /** What the value is, in the interface's own words. Omitted for a bare mark. */
  label?: string;
  value: string;
  title?: string;
  /** The trailing affordance: a caret for the readings that open a menu. */
  trailing?: ReactNode;
  /**
   * Present when the reading answers to a press, and then the whole thing is a
   * `<button>`. A `<span>` with an `onClick` -- what this used to be -- is a
   * control the keyboard cannot reach and a screen reader cannot find.
   */
  button?: ButtonHTMLAttributes<HTMLButtonElement> & { ref?: Ref<HTMLButtonElement> };
}

function Reading({ kind, label, value, title, trailing, button }: ReadingProps) {
  // `running` is the one status that is live rather than a verdict, so it is the
  // one mark that moves.
  const mark = <StatusDot status={kind} pulse={kind === 'running'} />;
  const content = (
    <>
      {mark}
      {label && <span className={styles.label}>{label}</span>}
      <span className={styles.value}>{value}</span>
      {trailing}
    </>
  );

  if (button) {
    return (
      <button
        {...button}
        type="button"
        data-ui="status-item"
        data-interactive="true"
        title={title}
        className={styles.item}
      >
        {content}
      </button>
    );
  }
  return (
    <span data-ui="status-item" title={title} className={styles.item}>
      {content}
    </span>
  );
}

/** A reading that reports and answers to nothing. */
export function StatusItem(props: Omit<ReadingProps, 'button' | 'trailing'>) {
  return <Reading {...props} />;
}

/**
 * A reading you can change: mode, thinking level, context budget.
 *
 * These three were antd `Dropdown`s wrapped around a `<span>`, which is the same
 * unreachable control in a different costume -- no `role`, no arrow keys, nothing
 * focusable. `Menu` brings the ARIA and the keyboard model, and this file keeps the
 * two things a *status bar* menu has to know: it opens upward, because it lives
 * against the bottom edge of the window, and its caret is the only hint that the
 * reading is a control at all.
 *
 * Disabling it is the caller's `disabled`, and it is honest: the core ignores a
 * mode change mid-turn, so a menu that opened and did nothing would be lying about
 * what it can do.
 */
export function StatusMenu({
  kind,
  label,
  value,
  title,
  options,
  onSelect,
  disabled = false,
}: {
  kind: ChipStatus;
  label?: string;
  value: string;
  title?: string;
  options: MenuItem[];
  onSelect: (key: string) => void;
  disabled?: boolean;
}) {
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const id = useId();
  const [open, setOpen] = useState(false);

  return (
    <>
      <Reading
        kind={kind}
        label={label}
        value={value}
        title={title}
        trailing={<Icon src={ChevronDown} className={styles.caret} />}
        button={{
          id,
          ref: anchorRef,
          'aria-haspopup': 'menu',
          'aria-expanded': open,
          disabled,
          onClick: () => setOpen((o) => !o),
        }}
      />
      <Menu
        open={open}
        anchorRef={anchorRef}
        onClose={() => setOpen(false)}
        labelledBy={id}
        side="top"
        align="start"
        items={options.map((option) => ({
          ...option,
          onSelect: () => onSelect(option.key),
        }))}
      />
    </>
  );
}

/** The separator between status groups. A 1px rule, not a border. */
export function StatusDivider() {
  return <span aria-hidden className={styles.divider} />;
}

/** Packs a reading against the right edge, past every other item. */
export function StatusTail({ children }: { children: ReactNode }) {
  return <span className={styles.tail}>{children}</span>;
}

export function StatusBar({ children }: { children: ReactNode }) {
  return (
    <footer data-ui="status-bar" className={styles.bar}>
      {children}
    </footer>
  );
}
