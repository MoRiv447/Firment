import { Alert, Button, Input, Space, Spin, Tag, Typography } from 'antd';
import { ArrowDownOutlined, SendOutlined, StopOutlined } from '@ant-design/icons';
import { useEffect, useRef, useState } from 'react';
import { MessageList, Markdown } from '../components/MessageList';
import { ToolCard } from '../components/ToolCard';
import { StepProgress } from '../components/StepProgress';
import { shouldShowStallNotice, stallNotice } from '../lib/stallHint';
import { workflowSteps } from '../lib/steps';
import type { RunningTurn, SessionDto } from '../types';
import { color, font, radius, statusChip } from '../styles/tokens';

const { Text } = Typography;
const { TextArea } = Input;

function fmtElapsed(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

export function ChatView({
  session,
  running,
  turn,
  infos,
  onSend,
  onCancel,
}: {
  session: SessionDto | null;
  running: boolean;
  turn: RunningTurn | null;
  infos: { id: number; text: string }[];
  onSend: (input: string) => void;
  onCancel: () => void;
}) {
  const [input, setInput] = useState('');
  const scrollRef = useRef<HTMLDivElement>(null);
  // Seconds since the last visible event, ticking every second while running —
  // the user can watch this climb to tell a slow model from a wedged turn.
  const [idleSecs, setIdleSecs] = useState(0);

  // Detect a stuck agent: running is on but nothing has changed in the
  // visible turn (text delta, new tool, or a tool finishing) for too long.
  // Tool STATUS transitions count as change: a 90s build is a running tool,
  // not a wedged turn. What does NOT count here is a model writing one huge
  // tool-call argument — the turn is alive downstream but invisible to this
  // key, which is why the notice waits until past the agent's own 120s
  // stream budget (see STALL_NOTICE_SECS) rather than crying at 60s.
  const lastChangeRef = useRef<number>(Date.now());
  const [stuck, setStuck] = useState(false);
  const notice = stallNotice(idleSecs);
  const turnKey = `${turn?.text.length}:${
    turn?.thinking.length ?? 0
  }:${turn?.tools ? Object.values(turn.tools).map((t) => t.status).join('') : ''}`;
  useEffect(() => {
    lastChangeRef.current = Date.now();
    setStuck(false);
    setIdleSecs(0);
  }, [turnKey]);
  useEffect(() => {
    if (!running) {
      setStuck(false);
      setIdleSecs(0);
      return;
    }
    const tick = setInterval(() => {
      const idle = Math.floor((Date.now() - lastChangeRef.current) / 1000);
      setIdleSecs(idle);
      if (shouldShowStallNotice(idle)) setStuck(true);
    }, 1000);
    return () => clearInterval(tick);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);

  // Stick-to-bottom scrolling: follow the stream only while the user is at
  // the bottom; scrolling up detaches (a jump pill appears) until they
  // re-engage. The scroll itself is coalesced into one rAF per change so a
  // delta burst costs one layout read+write, not one per event.
  const stickRef = useRef(true);
  const rafRef = useRef<number | null>(null);
  const [detached, setDetached] = useState(false);
  const followIfStuck = () => {
    if (!stickRef.current) return;
    const el = scrollRef.current;
    if (!el) return;
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
      rafRef.current = null;
    });
  };
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const stuck = el.scrollTop + el.clientHeight >= el.scrollHeight - 80;
    stickRef.current = stuck;
    setDetached(!stuck);
  };
  useEffect(() => {
    followIfStuck();
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [session?.messages.length, turn?.text, turn?.tools]);

  const send = () => {
    const trimmed = input.trim();
    if (!trimmed || running) return;
    setInput('');
    onSend(trimmed);
  };

  const toolList = turn ? Object.values(turn.tools) : [];
  // Where the embedded workflow got to, computed from those same tools rather
  // than tracked separately -- see `lib/steps.ts`. Null for a chat that never
  // builds anything, so the row does not appear as empty furniture.
  const steps = workflowSteps(toolList);
  const runningTools = toolList.filter((t) => t.status === 'running');
  const lastRunning = runningTools[runningTools.length - 1];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div
        ref={scrollRef}
        onScroll={onScroll}
        style={{
          flex: 1,
          overflow: 'auto',
          padding: '24px 28px',
          background: color.bg,
          position: 'relative',
        }}
      >
        {session && (
          <>
            <MessageList messages={session.messages} />
            {running && (
              <div style={{ margin: '12px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
                <Spin size="small" />
                <Text type="secondary" style={{ fontSize: 13 }}>
                  {lastRunning
                    ? `running ${lastRunning.name}…`
                    : turn && turn.text === ''
                      ? 'thinking…'
                      : 'generating…'}
                </Text>
                <Text type="secondary" style={{ fontSize: 12, fontFamily: font.mono }}>
                  {lastRunning
                    ? // The RUNNING TOOL's own elapsed, not time since the
                      // last visible event (a chatty stream used to keep
                      // resetting this to 0s and a finished wave kept it
                      // climbing under the final text phase).
                      `tool ${fmtElapsed(Date.now() - (lastRunning.startedAt ?? Date.now()))}`
                    : `idle ${fmtElapsed(idleSecs * 1000)}`}
                </Text>
                {idleSecs > 45 && (
                  <Tag
                    style={{
                      ...statusChip('attention'),
                      borderRadius: radius.chip,
                      fontWeight: 700,
                    }}
                  >
                    no events for {idleSecs}s
                  </Tag>
                )}
              </div>
            )}
            {infos.map((i) => (
              <Alert
                key={i.id}
                type="warning"
                showIcon
                style={{ margin: '8px 0', borderRadius: radius.control }}
                message={i.text}
              />
            ))}
            {stuck && (
              <Alert
                type="warning"
                showIcon
                style={{ margin: '8px 0', borderRadius: radius.control }}
                message={notice.message}
                description={notice.description}
              />
            )}
            {steps && (
              <div style={{ margin: '8px 0' }}>
                <StepProgress steps={steps} />
              </div>
            )}
            {toolList.map((t) => (
              <ToolCard key={t.seq} tool={t} onAction={onSend} />
            ))}
            {turn && turn.thinking && !turn.text && (
              <div
                style={{
                  marginTop: 10,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                  color: color.infoInk,
                  fontStyle: 'italic',
                  fontSize: 13,
                  lineHeight: 1.6,
                }}
              >
                💭 {turn.thinking.slice(-400)}
              </div>
            )}
            {turn && turn.thinking && turn.text && (
              <details style={{ marginTop: 8, color: color.infoInk, fontSize: 12 }}>
                <summary style={{ cursor: 'pointer', fontStyle: 'italic', userSelect: 'none' }}>
                  💭 reasoning…
                </summary>
                <div
                  style={{
                    whiteSpace: 'pre-wrap',
                    wordBreak: 'break-word',
                    fontStyle: 'italic',
                    marginTop: 4,
                    lineHeight: 1.6,
                  }}
                >
                  {turn.thinking.slice(-1200)}
                </div>
              </details>
            )}
            {turn && turn.text && (
              <div
                style={{
                  marginTop: 10,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                  color: color.ink,
                  lineHeight: 1.7,
                }}
              >
                {/* Same renderer as the committed transcript: the reply no
                    longer snaps from raw markdown to formatted at turn end. */}
                <Markdown>{turn.text}</Markdown>
              </div>
            )}
          </>
        )}
        {detached && (
          <Button
            size="small"
            icon={<ArrowDownOutlined />}
            onClick={() => {
              stickRef.current = true;
              const el = scrollRef.current;
              if (el) el.scrollTop = el.scrollHeight;
              setDetached(false);
            }}
            style={{
              position: 'absolute',
              bottom: 16,
              left: '50%',
              transform: 'translateX(-50%)',
              borderRadius: radius.control,
              border: `1px solid ${color.outline}`,
              boxShadow: color.shadowSm,
              fontWeight: 700,
              zIndex: 5,
            }}
          >
            跳到底部
          </Button>
        )}
        {!session && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              flex: 1,
              minHeight: 240,
              gap: 10,
              color: color.muted,
            }}
          >
            <div style={{ fontSize: 28, fontWeight: 800, letterSpacing: 0.5 }}>Firment</div>
            <div style={{ fontSize: 13 }}>Open or create a session from the sidebar to begin.</div>
          </div>
        )}
      </div>
      <div
        style={{
          padding: '14px 20px 16px',
          borderTop: `1px solid ${color.line}`,
          background: color.surface,
        }}
      >
        {session && (
          <Space size={6} style={{ marginBottom: 8, flexWrap: 'wrap' }}>
            <Tag
              color={color.brandAcid}
              style={{
                borderRadius: radius.chip,
                fontWeight: 700,
                border: `1px solid ${color.outline}`,
                boxShadow: color.shadowSm,
                color: color.onAcid,
              }}
            >
              {session.provider}
            </Tag>
            <Tag
              style={{
                borderRadius: radius.chip,
                border: `1px solid ${color.outline}`,
                color: color.ink,
                fontWeight: 600,
              }}
            >
              {session.model}
            </Tag>
            <Tag
              style={{
                // The fill used to be `warnInk`/`successInk` with `outline` as
                // the text: those inks are bright in the dark scheme and dark
                // in the light one, so this chip was readable in exactly one of
                // the two.
                ...statusChip(session.mode === 'plan' ? 'attention' : 'ok'),
                borderRadius: radius.chip,
                fontWeight: 700,
              }}
            >
              {session.mode}
            </Tag>
            <Tag
              style={{
                borderRadius: radius.chip,
                border: `1px solid ${color.outline}`,
                color: color.muted,
                fontFamily: font.mono,
                background: color.bg,
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
              background: color.bg,
              border: `1px solid ${color.outline}`,
              borderRadius: radius.control,
              boxShadow: color.shadowLg,
              color: color.ink,
              fontFamily: font.mono,
            }}
          />
          {running ? (
            <Button
              danger
              icon={<StopOutlined />}
              onClick={onCancel}
              style={{
                height: 'auto',
                borderRadius: radius.control,
                border: `1px solid ${color.outline}`,
                boxShadow: color.shadowLg,
                fontWeight: 700,
              }}
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
                borderRadius: radius.control,
                border: `1px solid ${color.outline}`,
                boxShadow: color.shadowLg,
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