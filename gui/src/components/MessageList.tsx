import { Alert, Space, Tag, Typography } from 'antd';
import ReactMarkdown from 'react-markdown';
import { ToolCard } from './ToolCard';
import type { ChatMessage, ToolCall } from '../types';

const { Text } = Typography;

function ToolCallBlock({ calls }: { calls: ToolCall[] }) {
  return (
    <Space direction="vertical" style={{ width: '100%' }} size={4}>
      {calls.map((c, i) => (
        <ToolCard
          key={i}
          tool={{ seq: i + 1, name: c.name, args: c.arguments, status: 'ok' }}
        />
      ))}
    </Space>
  );
}

export function MessageList({ messages }: { messages: ChatMessage[] }) {
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
                <ReactMarkdown
                  components={{
                    // ReactMarkdown's <p> carries a default 1em margin that
                    // adds a visible gap under short user messages inside the
                    // padded bubble; zero it so the bubble hugs its content.
                    p: ({ children }) => <div style={{ margin: 0 }}>{children}</div>,
                    ul: ({ children }) => (
                      <ul style={{ margin: 0, paddingLeft: 20 }}>{children}</ul>
                    ),
                    ol: ({ children }) => (
                      <ol style={{ margin: 0, paddingLeft: 20 }}>{children}</ol>
                    ),
                    pre: ({ children }) => <pre style={{ margin: '6px 0 0' }}>{children}</pre>,
                  }}
                >
                  {m.content}
                </ReactMarkdown>
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
              {m.content && <ReactMarkdown>{m.content}</ReactMarkdown>}
            </div>
          );
        }
        if (m.role === 'tool') {
          return (
            <div key={i} style={{ display: 'flex', justifyContent: 'flex-start', maxWidth: '92%' }}>
              <div style={{ width: '100%' }}>
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
                  tool // {m.name}
                </Tag>
                <Alert
                  type="info"
                  showIcon
                  style={{ borderRadius: 0, border: '2px solid #000' }}
                  message={
                    <Text style={{ fontSize: 12, whiteSpace: 'pre-wrap' }}>
                      {m.content.slice(0, 2000)}
                      {m.content.length > 2000 ? '…' : ''}
                    </Text>
                  }
                />
              </div>
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
}