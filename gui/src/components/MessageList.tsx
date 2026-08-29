import { memo, useState } from 'react';
import type { ReactNode } from 'react';
import { Alert, Space, Tag, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Components } from 'react-markdown';
import { ToolCard } from './ToolCard';
import type { ChatMessage, ToolCall } from '../types';

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
  pre: ({ children }) => <pre style={{ margin: '6px 0 0' }}>{children}</pre>,
  table: ({ children }) => (
    <table
      style={{
        borderCollapse: 'collapse',
        margin: '8px 0',
        border: '2px solid #000000',
        boxShadow: '3px 3px 0 #000000',
      }}
    >
      {children}
    </table>
  ),
  th: ({ children }) => (
    <th
      style={{
        border: '2px solid #000000',
        background: '#1a1e26',
        padding: '5px 10px',
        textAlign: 'left',
        fontWeight: 700,
      }}
    >
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td style={{ border: '2px solid #000000', padding: '4px 10px' }}>{children}</td>
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
        border: '2px solid #000000',
        background: '#12151b',
      }}
    >
      <RightOutlined style={{ fontSize: 9, color: '#9aa3b2' }} />
      <Tag color="green" style={{ borderRadius: 0, border: '2px solid #000', color: '#e6e9ef', fontWeight: 700 }}>
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
          border: '2px solid #000000',
          background: '#12151b',
        }}
      >
        {open ? (
          <DownOutlined style={{ fontSize: 9, color: '#9aa3b2' }} />
        ) : (
          <RightOutlined style={{ fontSize: 9, color: '#9aa3b2' }} />
        )}
        <Tag
          color="#facc15"
          style={{
            borderRadius: 0,
            border: '2px solid #000',
            color: '#000',
            fontWeight: 700,
            boxShadow: '2px 2px 0 #000',
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
          style={{ borderRadius: 0, border: '2px solid #000', borderTopWidth: 0 }}
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
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {messages.map((m, i) => {
        if (m.role === 'user') {
          return (
            <div key={i} style={{ display: 'flex', justifyContent: 'flex-end' }}>
              <div
                style={{
                  maxWidth: '80%',
                  background: '#2f6bff',
                  border: '3px solid #000000',
                  borderRadius: 0,
                  boxShadow: '4px 4px 0 #000000',
                  padding: '10px 16px',
                  lineHeight: 1.65,
                  color: '#ffffff',
                  fontWeight: 500,
                }}
              >
                <Markdown>{m.content}</Markdown>
              </div>
            </div>
          );
        }
        if (m.role === 'assistant') {
          return (
            <div key={i} style={{ maxWidth: '88%', lineHeight: 1.7 }}>
              {m.tool_calls && m.tool_calls.length > 0 && (
                <ToolCallBlock calls={m.tool_calls} />
              )}
              {m.content && <Markdown>{m.content}</Markdown>}
            </div>
          );
        }
        if (m.role === 'tool') {
          return (
            <div key={i} style={{ display: 'flex', justifyContent: 'flex-start', maxWidth: '92%' }}>
              <ToolResultCard name={m.name} content={m.content} />
            </div>
          );
        }
        return (
          <div key={i}>
            <Tag
              color="#a855f7"
              style={{ borderRadius: 0, border: '2px solid #000', color: '#000', fontWeight: 700 }}
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
