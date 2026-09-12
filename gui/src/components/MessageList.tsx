import { memo, useState } from 'react';
import type { ReactNode } from 'react';
import { Alert, Space, Tag, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Components } from 'react-markdown';
import { ToolCard } from './ToolCard';
import type { ChatMessage, ToolCall } from '../types';
import { color, radius, statusChip } from '../styles/tokens';
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

function ToolCallBlock({ calls }: { calls: ToolCall[] }) {
  return (
    <Space direction="vertical" style={{ width: '100%' }} size={4}>
      {calls.map((c, i) => (
        <CollapsedToolCard key={c.id || i} call={c} seq={i + 1} />
      ))}
    </Space>
  );
}

/// Historical tool calls start collapsed (name + one-line arg preview);
/// the header toggles open AND closed — expanding swaps in the full card
/// below it. Live cards during streaming stay expanded.
function CollapsedToolCard({ call, seq }: { call: ToolCall; seq: number }) {
  const [open, setOpen] = useState(false);
  let preview = '';
  try {
    preview = typeof call.arguments === 'string' ? call.arguments : JSON.stringify(call.arguments);
  } catch {
    preview = String(call.arguments ?? '');
  }
  if (preview.length > 90) preview = `${preview.slice(0, 90)}…`;
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
      {messages.map((m, i) => {
        // Stable key: index keys made expansion state migrate to the wrong
        // card whenever the optimistic-append → transcript-refresh cycle
        // shifted rows (compaction / interrupted-note inserts shift too).
        const key = `m${i}-${m.role}-${
          m.tool_call_id || m.tool_calls?.[0]?.id || m.content.slice(0, 24)
        }`;
        if (m.role === 'user') {
          return (
            <div key={key} style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <div
                style={{
                  maxWidth: '80%',
                  background: color.brandAcid,
                  border: `1px solid ${color.outline}`,
                  borderRadius: radius.tile,
                  boxShadow: color.shadowLg,
                  padding: '10px 16px',
                  lineHeight: 1.65,
                  color: color.ink,
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
