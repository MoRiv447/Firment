import { Fragment, memo, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, ChevronRight, Terminal } from 'lucide-react';

import type { ChatMessage, ToolCall, ToolCardState } from '../types';
import { pairRun, groupTranscript } from '../lib/transcript';
import { Chip, Icon, LogoMark, StatusMark } from '../ui';
import { Markdown } from './Markdown';
import { ToolCard } from './ToolCard';
import { ToolRow } from './ToolRow';
import styles from './MessageList.module.css';
import stack from './toolStack.module.css';

/**
 * A stored call, as both the row and the card want it.
 *
 * `status: 'unknown'` is not a placeholder -- see `ToolCard`'s header note. A stored
 * transcript keeps the call and the returned text and never whether the call worked,
 * so nothing here may claim an outcome. One function rather than two literals,
 * because a row and a card describing the same call must not disagree.
 */
function asTool(call: ToolCall, seq: number, result?: string): ToolCardState {
  return {
    seq,
    name: call.name,
    args: call.arguments,
    status: 'unknown',
    // A diff in the answer is rendered as a diff; anything else is the raw block,
    // which `ToolCard` picks by trying to parse it.
    detail: result ?? null,
  };
}

/**
 * The transcript: what the agent said, and the machinery rows folded away.
 *
 * Two things changed when this moved off antd and off the JS token module. The
 * obvious one is that the colours follow the scheme now -- every row used to be
 * painted from `styles/tokens.ts`, which is a snapshot of whichever palette was
 * cached when the file was first read, and `useThemeMode()` had to be called here
 * purely so this `memo` would notice a theme flip. That subscription is gone: CSS
 * custom properties re-resolve on their own.
 *
 * The second is what a row *is*. A run of tool work used to render two rows per
 * call -- the call card, then the result card holding the same text -- so a turn
 * that read four files and made two edits cost twelve lines to say "it went and
 * looked at things, then changed two files". One call is now one card, and the
 * result is the card's body.
 *
 * Where a row starts and ends is decided in `lib/transcript.ts`, not here. That
 * file is pure for the same reason the tests are cheap: the boundaries are the
 * design, and they are worth asserting without a renderer in the way.
 */

/** `read_file ×4 · edit_file` -- what the run did, not how many messages it took. */
function summariseRun(messages: ChatMessage[]): { steps: number; tools: string } {
  const calls = messages.flatMap((m) => m.tool_calls ?? []);
  const results = messages.filter((m) => m.role === 'tool');
  const counts = new Map<string, number>();
  for (const c of calls) counts.set(c.name, (counts.get(c.name) ?? 0) + 1);
  const tools = [...counts.entries()]
    .map(([name, n]) => (n > 1 ? `${name} ×${n}` : name))
    .join(' · ');
  return { steps: Math.max(calls.length, results.length), tools };
}

/**
 * A historical call: collapsed to its own head until opened.
 *
 * Its body is the result the transcript stored, which is why the fold exists --
 * a `read_file` answer is up to 2000 characters of file, and twelve of those
 * end-to-end is the reason nobody scrolled.
 */
function HistoryCard({
  call,
  result,
  seq,
  onAction,
  onOpenChanges,
}: {
  call: ToolCall;
  result?: string;
  seq: number;
  onAction?: (prompt: string) => void;
  onOpenChanges?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <ToolCard
      tool={asTool(call, seq, result)}
      collapsible={{ open, onToggle: () => setOpen((o) => !o) }}
      onAction={onAction} onOpenChanges={onOpenChanges}
    />
  );
}

/**
 * A tool answer no card claimed.
 *
 * Rare and worth keeping visible: the text is the output of something the agent
 * ran, and a transcript that silently loses it cannot be trusted to show the
 * rest either.
 */
function ResultRow({ message }: { message: ChatMessage }) {
  const [open, setOpen] = useState(false);
  const firstLine = message.content.split('\n')[0] ?? '';
  const preview = firstLine.length > 110 ? `${firstLine.slice(0, 110)}…` : firstLine || '(empty)';
  const lines = message.content.split('\n').length;
  return (
    <div data-ui="tool-result" className={styles.orphan}>
      <button
        type="button"
        className={styles.orphanHead}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon src={open ? ChevronDown : ChevronRight} size="sm" tone="muted" />
        <Chip status="neutral" size="sm" icon={Terminal}>
          {message.name ?? 'tool'}
        </Chip>
        {!open && <span className={styles.orphanPreview}>{preview}</span>}
        {!open && lines > 1 && <span className={styles.orphanLines}>{lines} lines</span>}
      </button>
      {open && <pre className={styles.orphanBody}>{message.content}</pre>}
    </div>
  );
}

/**
 * A run of tool work, folded to one band.
 *
 * This was a bare line -- `▸ 12 steps · read_file ×4 · edit_file ×2` with a hairline
 * running out to the right edge -- and it is a band now: an edge, a rounded corner,
 * a mark at the left, and the chevron moved to where the hairline used to end. A
 * hairline is what the interface uses *between* two rows of one list; a finished run
 * is not a row of the prose around it, it is a separate object that took time.
 *
 * **The mark is `pending`, and that is a measurement rather than a mood.** A stored
 * transcript keeps the call and the returned text and never whether the call worked
 * -- a denial and a timeout are both plain strings in a tool message -- which is why
 * `HistoryCard` builds every card with `status: 'unknown'` and why the band wears
 * `--border` rather than `--border-active`. An active edge here would be the
 * interface claiming a verdict it never recorded. See `ToolCard`'s header note: an
 * unknown call gets no mark and no green.
 */
export function ToolRun({
  messages,
  onAction,
  onOpenChanges,
}: {
  messages: ChatMessage[];
  onAction?: (prompt: string) => void;
  onOpenChanges?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const { steps, tools } = summariseRun(messages);
  const { entries, orphans } = pairRun(messages);
  return (
    <div data-ui="tool-run" className={styles.run}>
      <button
        type="button"
        className={styles.runHead}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <StatusMark state="pending" label="Outcome not recorded" />
        <span className={styles.runSteps}>
          {steps} step{steps === 1 ? '' : 's'}
        </span>
        {tools && <span className={styles.runTools}>{tools}</span>}
        <Icon src={open ? ChevronDown : ChevronRight} size="sm" tone="muted" className={styles.runChevron} />
      </button>
      {open && (
        <div className={styles.runBody}>
          {entries.map((entry, i) => (
            <HistoryCard
              key={entry.call.id ?? i}
              call={entry.call}
              result={entry.result}
              seq={i + 1}
              onAction={onAction} onOpenChanges={onOpenChanges}
            />
          ))}
          {orphans.map((message, i) => (
            <ResultRow key={message.tool_call_id ?? `o${i}`} message={message} />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Tool calls carried by a message that also has prose: the prose is the row, so the
 * calls sit above it rather than being folded into a run.
 *
 * They sit in a **ruled column of rows**, not a card each. A card per call was four
 * bordered boxes wedged between two paragraphs -- four times the chrome for the same
 * information, and it made one turn's work look like four unrelated things. The row
 * carries the name, the literal it was handed and the duration; pressing it opens
 * the card, which is where the two thousand characters of output belong.
 */
/**
 * One answer, framed the same way whether it is stored or still streaming.
 *
 * The tile carries the mark and the name line carries the label; everything the turn
 * produced sits in the column to its right, so the transcript's left edge is one
 * line of tiles rather than a ragged set of captions. Exported because `ChatView`
 * renders the live turn and must not re-derive the frame -- two copies of a grid is
 * how the two states quietly stop matching.
 */
export function AgentTurn({ children }: { children: ReactNode }) {
  return (
    <div className={styles.assistant}>
      <span aria-hidden className={styles.avatar}>
        <LogoMark />
      </span>
      <div className={styles.turn}>{children}</div>
    </div>
  );
}

function CallList({
  calls,
  onAction,
  onOpenChanges,
}: {
  calls: ToolCall[];
  onAction?: (prompt: string) => void;
  onOpenChanges?: () => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <div className={stack.stack}>
      {calls.map((call, i) => {
        const key = call.id ?? String(i);
        const tool = asTool(call, i + 1);
        return (
          <Fragment key={key}>
            <ToolRow
              tool={tool}
              open={open === key}
              onToggle={() => setOpen((s) => (s === key ? null : key))}
            />
            {open === key && (
              <div className={stack.card}>
                <ToolCard tool={tool} onAction={onAction} onOpenChanges={onOpenChanges} />
              </div>
            )}
          </Fragment>
        );
      })}
    </div>
  );
}

function SystemRow({ content }: { content: string }) {
  return (
    <div className={styles.system}>
      <Chip status="running" size="sm">
        system
      </Chip>
      <p className={styles.systemText}>{content}</p>
    </div>
  );
}

export const MessageList = memo(function MessageList({
  messages,
  onAction,
  onOpenChanges,
}: {
  messages: ChatMessage[];
  /** Sends a canned request to the agent -- the same path the composer uses. See
   *  lib/quickActions.ts. */
  onAction?: (prompt: string) => void;
  onOpenChanges?: () => void;
}): ReactNode {
  return (
    <div data-ui="transcript" className={styles.transcript}>
      {groupTranscript(messages).map((row) => {
        // `row.key` is derived from the message that starts the row, never from
        // the index alone -- index keys made expansion state migrate to the wrong
        // card when the optimistic-append → transcript-refresh cycle shifted rows.
        if (row.kind === 'run') {
          return <ToolRun key={row.key} messages={row.messages} onAction={onAction} onOpenChanges={onOpenChanges} />;
        }
        const m = row.message;
        const key = row.key;
        if (m.role === 'user') {
          return (
            // Plain text, not markdown: the user's pasted `snake_case` and
            // `#include` lines are code, not prose to italicize.
            <div key={key} data-ui="user-bubble" className={styles.bubble}>
              {m.content}
            </div>
          );
        }
        if (m.role === 'assistant') {
          return (
            <AgentTurn key={key}>
              {/* Who is speaking, in the machine voice, beside the tile that says the
                  same thing without words. Both are kept: the label is what a screen
                  reader reads, the tile is what the eye finds after a scroll. */}
              <span className={styles.who}>Firment</span>
              {!!m.tool_calls?.length && <CallList calls={m.tool_calls} onAction={onAction} onOpenChanges={onOpenChanges} />}
              {m.content && <Markdown>{m.content}</Markdown>}
            </AgentTurn>
          );
        }
        if (m.role === 'tool') {
          return <ResultRow key={key} message={m} />;
        }
        return <SystemRow key={key} content={m.content} />;
      })}
    </div>
  );
});
