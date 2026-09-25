import { useEffect, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';

import type { ToolCardState } from '../types';
import { Icon, StatusDot } from '../ui';
import type { ChipStatus } from '../ui';
import { ToolCard } from './ToolCard';
import { TurnTimeline } from './TurnTimeline';
import styles from './LiveRun.module.css';

/**
 * A live turn's tool work, folded to one line while it happens.
 *
 * The historical transcript folds a finished run to `▸ 12 steps · read_file ×4 ·
 * edit_file ×2` (see `MessageList`). A running one cannot use the same line,
 * because the question during a run is not "what did it do" but "what is it doing
 * *now*" -- so the header names the tool that is in flight and counts the seconds,
 * and everything already finished waits behind the fold.
 *
 * That is the difference between a progress line and a log. Five cards scrolling
 * past is a log; `⏺ edit_file · 12s · 4 steps` is a status, and it costs one row
 * whether the turn has taken two steps or forty.
 *
 * The state colour is a `StatusDot` rather than a swatch of `statusChip().color`:
 * the chip helper returned a colour string for an inline style, which is the
 * exact shape of the bug that kept a run painted in the previous scheme's green.
 */

/** `read_file ×4 · edit_file` -- the shape of the work, not its length. */
function toolCounts(tools: ToolCardState[]): string {
  const counts = new Map<string, number>();
  for (const t of tools) counts.set(t.name, (counts.get(t.name) ?? 0) + 1);
  return [...counts.entries()]
    .map(([name, n]) => (n > 1 ? `${name} ×${n}` : name))
    .join(' · ');
}

export function LiveRun({
  tools,
  onAction,
  turnStartedAt,
}: {
  tools: ToolCardState[];
  onAction?: (prompt: string) => void;
  /**
   * When the turn began, which is earlier than the first card: the stretch between
   * the two is the model answering, and a timeline that started at the first call
   * would quietly drop the longest part of some turns.
   */
  turnStartedAt?: number;
}) {
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  const sorted = [...tools].sort((a, b) => a.seq - b.seq);
  const busy = sorted.some((t) => t.status === 'running');

  // One tick per second, and only while something is in flight: a finished run
  // has nothing to count and must not re-render the transcript.
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [busy]);

  if (sorted.length === 0) return null;

  const started = sorted.find((t) => t.startedAt)?.startedAt;
  const seconds = started ? Math.max(0, Math.round((now - started) / 1000)) : null;
  const current = [...sorted].reverse().find((t) => t.status === 'running');
  const failed = sorted.filter((t) => t.status === 'failed').length;

  // The dot reports the run, not the individual step: a run with a failure in it
  // is a failed run even if the steps after it succeeded, and a run with
  // something in flight is neither.
  const status: ChipStatus = busy ? 'running' : failed > 0 ? 'failed' : 'ok';

  return (
    <div data-ui="live-run" className={styles.root}>
      <button
        type="button"
        className={styles.head}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon src={open ? ChevronDown : ChevronRight} size="sm" tone="muted" />
        <StatusDot status={status} pulse={busy} />
        {current ? (
          // The tool in flight, by name. This is the one piece of the run that is
          // worth a permanent line.
          <span className={styles.current}>{current.name}</span>
        ) : (
          <span className={styles.steps}>{sorted.length} step{sorted.length === 1 ? '' : 's'}</span>
        )}
        {seconds !== null && <span className={styles.steps}>{seconds}s</span>}
        {sorted.length > 1 && (
          <span className={styles.tools}>
            {sorted.length} steps · {toolCounts(sorted)}
          </span>
        )}
        <span aria-hidden className={styles.rule} />
      </button>
      {open && (
        <div className={styles.body}>
          {/*
            * The summary of where the time went, above the things it was spent on.
            * Inside the fold rather than on the run line: the line's whole design is
            * that it costs one row however long the turn gets, and this is a detail
            * for the moment someone has decided to look.
            */}
          <TurnTimeline tools={sorted} now={now} turnStartedAt={turnStartedAt} />
          {sorted.map((t) => (
            <ToolCard key={t.seq} tool={t} onAction={onAction} />
          ))}
        </div>
      )}
    </div>
  );
}
