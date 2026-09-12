import { useEffect, useState } from 'react';
import { font, radius, color, space, statusChip } from '../styles/tokens';
import { ToolCard } from './ToolCard';
import type { ToolCardState } from '../types';

/**
 * A live turn's tool work, folded to one line while it happens.
 *
 * The historical transcript folds a finished run to `▸ 2 步 · read_file · edit_file`
 * (see `MessageList`). A running one cannot use the same line, because the
 * question during a run is not "what did it do" but "what is it doing *now*" --
 * so the header names the tool that is in flight and counts the seconds, and
 * everything already finished waits behind the fold.
 *
 * That is the difference between a progress line and a log. Five cards scrolling
 * past is a log; `⏺ edit_file · 12s · 4 步` is a status, and it costs one row
 * whether the turn has taken two steps or forty.
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
}: {
  tools: ToolCardState[];
  onAction?: (prompt: string) => void;
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

  // The dot reports the session, not the individual step: a run with a failure
  // in it is a failed run even if the steps after it succeeded, and a run with
  // something in flight is neither.
  const chip = statusChip(busy ? 'running' : failed > 0 ? 'failed' : 'ok');

  return (
    <div style={{ width: '100%', margin: '6px 0' }}>
      <div
        onClick={() => setOpen((o) => !o)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') setOpen((o) => !o);
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          cursor: 'pointer',
          padding: '3px 0',
          fontFamily: font.sans,
          fontSize: 12,
        }}
      >
        <span style={{ fontSize: 9, color: color.muted }}>{open ? '▾' : '▸'}</span>
        <span
          aria-hidden
          style={{
            width: 6,
            height: 6,
            flex: '0 0 auto',
            borderRadius: radius.chip,
            background: chip.color,
          }}
        />
        {current ? (
          // The tool in flight, by name. This is the one piece of the run that
          // is worth a permanent line.
          <span style={{ color: color.ink, fontFamily: font.mono }}>{current.name}</span>
        ) : (
          <span style={{ color: color.muted }}>{sorted.length} 步</span>
        )}
        {seconds !== null && (
          <span style={{ color: color.muted, fontFamily: font.mono }}>{seconds}s</span>
        )}
        {sorted.length > 1 && (
          <span
            style={{
              color: color.muted,
              fontFamily: font.mono,
              fontSize: 11,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {sorted.length} 步 · {toolCounts(sorted)}
          </span>
        )}
        <span aria-hidden style={{ flex: 1, height: 1, background: color.line, minWidth: 12 }} />
      </div>
      {open && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.controlGap, paddingTop: 6 }}>
          {sorted.map((t) => (
            <ToolCard key={t.seq} tool={t} onAction={onAction} />
          ))}
        </div>
      )}
    </div>
  );
}
