import { Alert, Button, Input, Space, Spin, Tag, Typography } from 'antd';
import { SendOutlined, StopOutlined } from '@ant-design/icons';
import { useEffect, useRef, useState } from 'react';
import { MessageList } from '../components/MessageList';
import { ToolCard } from '../components/ToolCard';
import type { RunningTurn, SessionDto } from '../types';

const { Text } = Typography;
const { TextArea } = Input;

export function ChatView({
  session,
  running,
  turn,
  onSend,
  onCancel,
}: {
  session: SessionDto | null;
  running: boolean;
  turn: RunningTurn | null;
  onSend: (input: string) => void;
  onCancel: () => void;
}) {
  const [input, setInput] = useState('');
  const scrollRef = useRef<HTMLDivElement>(null);

  // Detect a stuck agent: running is on but nothing has changed in the
  // visible turn (text delta or new tool) for too long. Cheap proxy for
  // "the provider stream is dead but Rust has not yet emitted TurnEnd" —
  // shows a banner telling the user to hit Stop instead of waiting forever.
  const lastChangeRef = useRef<number>(Date.now());
  const [stuck, setStuck] = useState(false);
  const turnKey = `${turn?.text.length}:${turn?.tools ? Object.keys(turn.tools).length : 0}`;
  useEffect(() => {
    lastChangeRef.current = Date.now();
    setStuck(false);
  }, [turnKey]);
  useEffect(() => {
    if (!running) {
      setStuck(false);
      return;
    }
    const tick = setInterval(() => {
      if (Date.now() - lastChangeRef.current > 60_000) setStuck(true);
    }, 10_000);
    return () => clearInterval(tick);
  }, [running]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [session?.messages.length, turn?.text, turn?.tools]);

  const send = () => {
    const trimmed = input.trim();
    if (!trimmed || running) return;
    setInput('');
    onSend(trimmed);
  };

  const toolList = turn ? Object.values(turn.tools) : [];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflow: 'auto',
          padding: '24px 28px',
          background: '#0a0c10',
        }}
      >
        {session && (
          <>
            <MessageList messages={session.messages} />
            {running && (
              <div style={{ margin: '12px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
                <Spin size="small" />
                <Text type="secondary" style={{ fontSize: 13 }}>
                  {toolList.length > 0
                    ? `running ${toolList[toolList.length - 1].name}…`
                    : turn && turn.text === ''
                      ? 'thinking…'
                      : 'generating…'}
                </Text>
              </div>
            )}
            {stuck && (
              <Alert
                type="warning"
                showIcon
                style={{ margin: '8px 0', borderRadius: 0 }}
                message="Agent has been running with no new events for 60s."
                description="The provider stream may be stalled. Click Stop below to cancel the turn — your input will be re-enabled."
              />
            )}
            {toolList.map((t) => (
              <ToolCard key={t.seq} tool={t} />
            ))}
            {turn && turn.text && (
              <div
                style={{
                  marginTop: 10,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                  color: '#f2f4f8',
                  lineHeight: 1.7,
                }}
              >
                {turn.text}
              </div>
            )}
          </>
        )}
        {!session && (
          <div style={{ display: 'flex', justifyContent: 'center', marginTop: 80 }}>
            <Spin tip="Loading session…" />
          </div>
        )}
      </div>
      <div style={{ padding: '14px 20px 16px', borderTop: '3px solid #000000', background: '#12151b' }}>
        {session && (
          <Space size={6} style={{ marginBottom: 8, flexWrap: 'wrap' }}>
            <Tag
              color="#2f6bff"
              style={{ borderRadius: 0, fontWeight: 700, border: '2px solid #000', boxShadow: '2px 2px 0 #000' }}
            >
              {session.provider}
            </Tag>
            <Tag style={{ borderRadius: 0, border: '2px solid #000', color: '#e6e9ef', fontWeight: 600 }}>
              {session.model}
            </Tag>
            <Tag
              color={session.mode === 'plan' ? '#facc15' : '#22c55e'}
              style={{ borderRadius: 0, border: '2px solid #000', color: '#000', fontWeight: 700 }}
            >
              {session.mode}
            </Tag>
            <Tag
              style={{
                borderRadius: 0,
                border: '2px solid #000',
                color: '#9aa3b2',
                fontFamily: 'Consolas, monospace',
                background: '#0a0c10',
              }}
            >
              {session.cwd}
            </Tag>
          </Space>
        )}
        <Space.Compact style={{ width: '100%' }}>
          <TextArea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onPressEnter={(e) => {
              if (!e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder="Ask the agent… (Enter to send, Shift+Enter for newline)"
            autoSize={{ minRows: 2, maxRows: 8 }}
            disabled={running || !session}
            style={{
              fontSize: 14,
              background: '#0a0c10',
              border: '3px solid #000000',
              borderRadius: 0,
              boxShadow: '4px 4px 0 #000000',
              color: '#f2f4f8',
              fontFamily: "'JetBrains Mono', Consolas, monospace",
            }}
          />
          {running ? (
            <Button
              danger
              icon={<StopOutlined />}
              onClick={onCancel}
              style={{ height: 'auto', borderRadius: 0, border: '3px solid #000', boxShadow: '4px 4px 0 #000', fontWeight: 700 }}
            >
              Stop
            </Button>
          ) : (
            <Button
              type="primary"
              icon={<SendOutlined />}
              onClick={send}
              disabled={!session || !input.trim()}
              style={{
                height: 'auto',
                borderRadius: 0,
                border: '3px solid #000',
                boxShadow: '4px 4px 0 #000',
                fontWeight: 700,
              }}
            >
              Send
            </Button>
          )}
        </Space.Compact>
      </div>
    </div>
  );
}