import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { Icon } from './Icon';
import type { ChipStatus } from './types';
import styles from './Chip.module.css';

/**
 * A status chip: one word about one thing.
 *
 * The fill and the ink always come from the same documented pair, which is the
 * fix for how this used to be assembled -- `color={color.warnInk}` as the fill
 * with `color.outline` as the text. That survives a dark scheme where every ink
 * is bright and stops surviving a light one, where `successInk` is a dark green
 * and the same line of code paints a dark badge with dark writing in it.
 *
 * `[data-status]` does the pairing in CSS, so a chip cannot be built out of two
 * tokens that were measured against different grounds. It is also the reason
 * `statusChip()` in the old token layer can die with antd: the mapping stops
 * being a function someone has to remember to call.
 *
 * Borderless on purpose -- the neutral layer already spends its hairlines on
 * structure, and outlining every badge too is what made the old header read as
 * five competing boxes.
 *
 * There is no `className`. The one thing a parent legitimately needs from a chip
 * is already in the stylesheet: it shrinks and truncates rather than pushing the
 * row apart, so a long branch name cannot widen a header.
 */
export function Chip({
  status = 'neutral',
  icon,
  mono = false,
  size = 'md',
  children,
  title,
}: {
  status?: ChipStatus;
  icon?: LucideIcon;
  /** Monospace: a chip holding a path, a branch, a commit sha. */
  mono?: boolean;
  size?: 'sm' | 'md';
  children: ReactNode;
  title?: string;
}) {
  return (
    <span
      data-ui="chip"
      data-status={status}
      data-size={size === 'sm' ? 'sm' : undefined}
      data-mono={mono || undefined}
      title={title}
      className={styles.chip}
    >
      {icon ? <Icon src={icon} tone="inherit" /> : null}
      <span className={styles.text}>{children}</span>
    </span>
  );
}
