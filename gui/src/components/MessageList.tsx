import { memo, useState } from 'react';
import type { ReactNode } from 'react';
import { Alert, Space, Tag, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Components } from 'react-markdown';
import { ToolCard } from './ToolCard';
import { describeArgs } from '../lib/toolArgs';
import type { ChatMessage, ToolCall } from '../types';
import { color, font, radius, statusChip } from '../styles/tokens';
import { useThemeMode } from '../lib/theme';

const { Text } = Typography;

// Shared markdown pipeline: GFM is what makes pipe tables render as real
// tables — react-markdown does NOT support them by default, and without it
// a finished table collapsed into one long line of raw pipes.
const remarkPlugins = [remarkGfm];

const mdComponents: Components = {
  // ReactMarkdown's <p> carries a default 1em margin that adds a visible
  // gap under short messages; zero it.
  p: ({ children }) => <div style={{ margin: 0 }}>{children}</div>,
  ul: ({ children }) => <ul style={{ margin: 0, paddingLeft: 20 }}>{children}</ul>,
  ol: ({ children }) => <ol style={{ margin: 0, paddingLeft: 20 }}>{children}</ol>,
  pre: ({ children }) => (
    <pre
      style={{
        margin: '6px 0 0',
        // Long code lines used to push a horizontal scrollbar onto the WHOLE
        // chat scroll container — scroll inside the block instead.
        overflowX: 'auto',
        background: color.bg,
        border: `1px solid ${color.outline}`,
        padding: 8,
      }}
    >
      {children}
    </pre>
  ),
  code: ({ className, children, ...rest }) => {
    // Fenced blocks render code inside pre>code: only INLINE code gets the
    // pill background (the pre above already styles the block).
    const inline = !String(className || '').includes('language-');
    if (!inline) {
      return (
        <code className={className} {...rest}>
          {children}
        </code>
      );
    }
    return (
      <code
        className={className}
        {...rest}
        style={{
          background: color.surfaceRaised,
          padding: '1px 5px',
          border: `1px solid ${color.outline}`,
        }}
      >
        {children}
      </code>
    );
  },
  table: ({ children }) => (
    <table
      style={{
        borderCollapse: 'collapse',
        margin: '8px 0',
        border: `1px solid ${color.outline}`,
        boxShadow: color.shadowMd,
      }}
    >
      {children}
    </table>
  ),
  th: ({ children }) => (
    <th
      style={{
        border: `1px solid ${color.outline}`,
        background: color.surfaceRaised,
        padding: '5px 10px',
        textAlign: 'left',
        fontWeight: 700,
      }}
    >
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td style={{ border: `1px solid ${color.outline}`, padding: '4px 10px' }}>{children}</td>
  ),
};

export function Markdown({ children }: { children: string }) {
  return (
    <ReactMarkdown remarkPlugins={remarkPlugins} components={mdComponents}>
      {children}
    </ReactMarkdown>
  );
}

function ToolCallBlock({ calls, startSeq = 0 }: { calls: ToolCall[]; startSeq?: number }) {
  return (
    <Space direction="vertical" style={{ width: '100%' }} size={4}>
      {calls.map((c, i) => (
        <CollapsedToolCard key={c.id || i} call={c} seq={startSeq + i + 1} />
      ))}
    </Space>
  );
}

/**
 * One entry in the transcript after grouping.
 *
 * The unit of the transcript used to be the *message*, so a turn that read four
 * files and made two edits produced twelve rows: six tool-call cards and six
 * result cards, none of which anybody reads. A run of tool work is one event in
 * the story ("it went and looked at things"), so it is one row that opens.
 */
export type TranscriptRow =
  | { kind: 'single'; key: string; message: ChatMessage }
  | { kind: 'run'; key: string; messages: ChatMessage[] };

/** Assistant messages that only carry tool calls, and the results answering
 *  them. A message with prose is never part of a run: the prose is the point. */
function isRunMember(m: ChatMessage): boolean {
  if (m.role === 'tool') return true;
  return m.role === 'assistant' && !!m.tool_calls?.length && !m.content.trim();
}

/**
 * Collapse consecutive tool work into single rows, everything else untouched.
 *
 * Pure, so the shape can be asserted without rendering: the interesting cases
 * are the boundaries -- a run interrupted by a paragraph of prose is two runs,
 * not one, because the prose is what the reader is looking for.
 */
export function groupTranscript(messages: ChatMessage[]): TranscriptRow[] {
  const rows: TranscriptRow[] = [];
  let run: ChatMessage[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const first = run[0];
    const key = `run-${first.tool_calls?.[0]?.id ?? first.tool_call_id ?? rows.length}`;
    rows.push({ kind: 'run', key, messages: run });
    run = [];
  };
  messages.forEach((m, i) => {
    if (isRunMember(m)) {
      run.push(m);
      return;
    }
    flush();
    rows.push({ kind: 'single', key: `m${i}-${m.role}`, message: m });
  });
  flush();
  return rows;
}

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
 * A run of tool work, folded to one line.
 *
 * The line is the whole point: `› 12 步 · read_file ×4 · edit_file ×2` costs one
 * row where the messages cost twelve, and the transcript goes back to reading as
 * prose with machinery behind it. Nothing is hidden that cannot be opened in one
 * click.
 */
export function ToolRun({ messages }: { messages: ChatMessage[] }) {
  const [open, setOpen] = useState(false);
  const { steps, tools } = summariseRun(messages);
  let seq = 0;
  return (
    <div style={{ width: '100%' }}>
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
          color: color.muted,
          fontFamily: font.sans,
          fontSize: 12,
        }}
      >
        <span style={{ fontSize: 9 }}>{open ? '▾' : '▸'}</span>
        <span>{steps} 步</span>
        {tools && (
          <>
            <span aria-hidden style={{ opacity: 0.5 }}>
              ·
            </span>
            <span
              style={{
                fontFamily: font.mono,
                fontSize: 11,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {tools}
            </span>
          </>
        )}
        <span
          aria-hidden
          style={{ flex: 1, height: 1, background: color.line, minWidth: 12 }}
        />
      </div>
      {open && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, paddingTop: 6 }}>
          {messages.map((m, i) => {
            const key = `r${i}-${m.tool_call_id || m.tool_calls?.[0]?.id || i}`;
            if (m.role === 'assistant') {
              return <ToolCallBlock key={key} calls={m.tool_calls ?? []} startSeq={seq++} />;
            }
            return (
              <div key={key} style={{ display: 'flex', justifyContent: 'flex-start' }}>
                <ToolResultCard name={m.name} content={m.content} />
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/// Historical tool calls start collapsed (name + one-line arg preview);
/// the header toggles open AND closed — expanding swaps in the full card
/// below it. Live cards during streaming stay expanded.
function CollapsedToolCard({ call, seq }: { call: ToolCall; seq: number }) {
  const [open, setOpen] = useState(false);
  const preview = describeArgs(call.arguments);
  if (open) {
    // Expanded: the ToolCard itself carries the chevron in its title and
    // collapses on click — rendering our own header too would duplicate the
    // tool-name tag.
    return (
      <ToolCard
        tool={{ seq, name: call.name, args: call.arguments, status: 'ok' }}
        standalone
        collapsible={{ open: true, onToggle: () => setOpen(false) }}
      />
    );
  }
  return (
    <div
      onClick={() => setOpen(true)}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        cursor: 'pointer',
        padding: '4px 10px',
        // Full width like every sibling. Without this the row sized to its
        // content, so a card whose preview happened to be longer came out wider
        // than the one above it -- four tool rows, four different right edges.
        // `ToolResultCard` below already wraps for the same reason.
        width: '100%',
        border: `1px solid ${color.outline}`,
        background: color.surface,
      }}
    >
      <RightOutlined style={{ fontSize: 9, color: color.muted }} />
      <Tag
        style={{
          ...statusChip('ok'),
          borderRadius: radius.chip,
          fontWeight: 700,
        }}
      >
        ✓ {call.name}
      </Tag>
      <Text type="secondary" style={{ fontSize: 12, flex: 1, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
        {preview}
      </Text>
    </div>
  );
}

/// Historical tool RESULT: stored content can carry a huge spill excerpt
/// (up to 2000 chars of raw output kept inline by the backend), so these
/// start collapsed behind a one-line header too.
function ToolResultCard({ name, content }: { name?: string; content: string }) {
  const [open, setOpen] = useState(false);
  const firstLine = content.split('\n')[0] ?? '';
  const preview =
    firstLine.length > 110 ? `${firstLine.slice(0, 110)}…` : firstLine || '(empty)';
  const lines = content.split('\n').length;
  return (
    <div style={{ width: '100%' }}>
      <div
        onClick={() => setOpen((o) => !o)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          cursor: 'pointer',
          padding: '4px 10px',
          border: `1px solid ${color.outline}`,
          background: color.surface,
        }}
      >
        {open ? (
          <DownOutlined style={{ fontSize: 9, color: color.muted }} />
        ) : (
          <RightOutlined style={{ fontSize: 9, color: color.muted }} />
        )}
        <Tag
          style={{
            ...statusChip('attention'),
            borderRadius: radius.chip,
            fontWeight: 700,
            boxShadow: color.shadowSm,
          }}
        >
          tool // {name}
        </Tag>
        {!open && (
          <Text type="secondary" style={{ fontSize: 12, flex: 1, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
            {preview}
          </Text>
        )}
        {!open && lines > 1 && (
          <Text type="secondary" style={{ fontSize: 11 }}>
            {lines} lines
          </Text>
        )}
      </div>
      {open && (
        <Alert
          type="info"
          showIcon
          style={{
            borderRadius: radius.chip,
            border: `1px solid ${color.outline}`,
            borderTopWidth: 0,
          }}
          message={
            <Text style={{ fontSize: 12, whiteSpace: 'pre-wrap' }}>
              {content.slice(0, 2000)}
              {content.length > 2000 ? '…' : ''}
            </Text>
          }
        />
      )}
    </div>
  );
}

// Memoized: `messages` is referentially stable while a turn streams (only
// the live-turn state changes), so without this every text delta re-parsed
// the WHOLE transcript through react-markdown.
export const MessageList = memo(function MessageList({
  messages,
}: {
  messages: ChatMessage[];
}): ReactNode {
  // Subscription only, no value used: memo compares props, and `messages` does
  // not change when the colour scheme does, so without a context read here this
  // subtree would keep painting the previous scheme's colours. Reading the
  // context is what makes the memo follow the theme.
  useThemeMode();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {groupTranscript(messages).map((row) => {
        // Runs are folded to one row; everything else keeps the per-message
        // rendering it had. `row.key` is derived from the message that starts
        // the row, never from the index alone -- index keys made expansion
        // state migrate to the wrong card when the optimistic-append →
        // transcript-refresh cycle shifted rows.
        if (row.kind === 'run') {
          return <ToolRun key={row.key} messages={row.messages} />;
        }
        const m = row.message;
        const key = row.key;
        if (m.role === 'user') {
          return (
            <div key={key} style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <div
                style={{
                  maxWidth: '80%',
                  background: color.brandAcid,
                  borderRadius: radius.tile,
                  padding: '10px 16px',
                  lineHeight: 1.65,
                  // `onAcid`, not `ink`. This is the acid fill, so the text on
                  // it is the one token measured against acid (13.28:1). `ink`
                  // is near-white in the dark scheme, so the user's own message
                  // was rendering at about 1.3:1 -- measured, not guessed: 320
                  // near-white pixels inside the bubble in dark, 0 in light.
                  //
                  // No border and no shadow: the fill already separates the
                  // bubble from the transcript, and a grey ring around a green
                  // block is what made every filled control look like a mistake.
                  color: color.onAcid,
                  fontWeight: 500,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}
              >
                {/* Plain text: the user's pasted snake_case / #include lines
                    are code, not prose to italicize. */}
                {m.content}
              </div>
            </div>
          );
        }
        if (m.role === 'assistant') {
          return (
            <div key={key} style={{ maxWidth: '88%', lineHeight: 1.7 }}>
              {m.tool_calls && m.tool_calls.length > 0 && (
                <ToolCallBlock calls={m.tool_calls} />
              )}
              {m.content && <Markdown>{m.content}</Markdown>}
            </div>
          );
        }
        if (m.role === 'tool') {
          return (
            <div key={key} style={{ display: 'flex', justifyContent: 'flex-start', maxWidth: '92%' }}>
              <ToolResultCard name={m.name} content={m.content} />
            </div>
          );
        }
        return (
          <div key={key}>
            <Tag
              style={{
                ...statusChip('running'),
                borderRadius: radius.chip,
                fontWeight: 700,
              }}
            >
              system
            </Tag>
            <Text type="secondary" style={{ fontSize: 12, whiteSpace: 'pre-wrap' }}>
              {m.content}
            </Text>
          </div>
        );
      })}
    </div>
  );
});
