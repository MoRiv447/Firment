import { useEffect, useRef, useState } from 'react';
import { Button, ConfigProvider, Layout, Menu, Tag, theme } from 'antd';
import {
  ApiOutlined,
  MessageOutlined,
  RocketOutlined,
  TeamOutlined,
  UsbOutlined,
} from '@ant-design/icons';
import {
  api,
  onAgentEvent,
  onAskRequest,
  onMonitorExited,
  onMonitorOutput,
  onPermissionRequest,
} from './lib/api';
import type {
  AskRequest,
  MonitorLine,
  PermissionRequest,
  RunningTurn,
  SessionDto,
  SessionSummaryDto,
} from './types';
import { AskDialog, PermissionDialog } from './components/Dialogs';
import { ChatView } from './views/ChatView';
import { SessionSidebar } from './views/SessionSidebar';
import { SettingsView } from './views/SettingsView';
import { SerialView } from './views/SerialView';
import { FlashView } from './views/FlashView';
import { CollabView } from './views/CollabView';

const { Sider, Header, Content } = Layout;

type ViewKey = 'chat' | 'settings' | 'serial' | 'flash' | 'collab';

export default function App() {
  const [sessions, setSessions] = useState<SessionSummaryDto[]>([]);
  const [session, setSession] = useState<SessionDto | null>(null);
  const [running, setRunning] = useState(false);
  const [turn, setTurn] = useState<RunningTurn | null>(null);
  const [view, setView] = useState<ViewKey>('chat');
  const [permReq, setPermReq] = useState<PermissionRequest | null>(null);
  const [askReq, setAskReq] = useState<AskRequest | null>(null);
  const [monitorLines, setMonitorLines] = useState<Record<string, MonitorLine[]>>({});
  const [workCwd, setWorkCwd] = useState('C:\\');
  const monRef = useRef(monitorLines);
  monRef.current = monitorLines;
  // The event listeners are registered once ([] deps), so the closure would
  // otherwise capture the FIRST render's `session` (null) forever. Keep a ref
  // to the latest session so turn_end can refresh the transcript.
  const sessionRef = useRef(session);
  sessionRef.current = session;

  useEffect(() => {
    void api.listSessions().then(setSessions).catch(console.error);
  }, []);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      onAgentEvent((e) => {
        switch (e.type) {
          case 'turn_start':
            setRunning(true);
            setTurn({ text: '', tools: {}, startedAt: Date.now() });
            break;
          case 'text_delta':
            setTurn((t) => (t ? { ...t, text: t.text + e.text } : t));
            break;
          case 'tool_start':
            setTurn((t) =>
              t
                ? {
                    ...t,
                    tools: {
                      ...t.tools,
                      [e.seq]: { seq: e.seq, name: e.name, args: e.args, status: 'running' },
                    },
                  }
                : t,
            );
            break;
          case 'tool_end':
            setTurn((t) =>
              t && t.tools[e.seq]
                ? {
                    ...t,
                    tools: {
                      ...t.tools,
                      [e.seq]: {
                        ...t.tools[e.seq],
                        status: e.ok ? 'ok' : 'failed',
                        summary: e.summary,
                      },
                    },
                  }
                : t,
            );
            break;
          case 'turn_end':
            setRunning(false);
            // Clear the streaming turn so the transcript (refreshed below)
            // is the single source of truth — otherwise the same reply shows
            // twice (once in turn.text, once in session.messages).
            setTurn(null);
            const sid = sessionRef.current?.id;
            if (sid) {
              void api.sessionTranscript(sid).then(setSession).catch(console.error);
            }
            break;
          case 'session_loaded':
            setSession(e.session);
            break;
          case 'sessions':
            setSessions(e.sessions);
            break;
          case 'info':
            console.info('[firm]', e.message);
            break;
          case 'error':
            setTurn((t) => (t ? { ...t, text: `${t.text}\n⚠ ${e.message}` } : t));
            setRunning(false);
            break;
          default:
            break;
        }
      }),
    );

    unlisteners.push(
      onPermissionRequest((req) => setPermReq(req)),
      onAskRequest((req) => setAskReq(req)),
      onMonitorOutput((line) => {
        setMonitorLines((prev) => ({
          ...prev,
          [line.port]: [...(prev[line.port] ?? []), line].slice(-2000),
        }));
      }),
      onMonitorExited(({ port }) => {
        setMonitorLines((prev) => ({
          ...prev,
          [port]: [...(prev[port] ?? []), { port, kind: 'stderr', line: '── monitor exited ──' }],
        }));
      }),
    );

    return () => {
      void Promise.all(unlisteners.map((p) => p.then((u) => u())));
    };
  }, []);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (session) {
      void api.listSessions().then(setSessions).catch(console.error);
    }
  }, [session?.id]);

  const handleNewSession = async (mode: 'agent' | 'plan') => {
    try {
      const s = await api.newSession(workCwd || 'C:\\', mode);
      setSession(s);
      setView('chat');
      void api.listSessions().then(setSessions);
    } catch (err) {
      console.error(err);
    }
  };

  const handleSelectSession = async (id: string) => {
    try {
      const s = await api.loadSession(id);
      setSession(s);
      setView('chat');
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteSession = async (id: string) => {
    try {
      await api.deleteSession(id);
      void api.listSessions().then(setSessions);
    } catch (err) {
      console.error(err);
    }
  };

  const handleSend = (input: string) => {
    const snapshot = sessionRef.current?.messages ?? [];
    // 乐观追加用户消息，发送后立即显示在聊天区（turn_end 后以 transcript 为准）
    setSession((s) =>
      s ? { ...s, messages: [...s.messages, { role: 'user', content: input }] } : s,
    );
    void api.startTurn(input).catch((err) => {
      console.error(err);
      // 发送失败（如 agent 正忙）：回滚乐观消息，避免界面上出现"幽灵消息"
      setSession((s) => (s ? { ...s, messages: snapshot } : s));
    });
  };

  const handleCancel = () => {
    void api.cancelTurn();
  };

  return (
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorPrimary: '#2f6bff',
          borderRadius: 0,
          colorBgLayout: '#0a0c10',
          colorBgContainer: '#12151b',
          colorBgElevated: '#1a1e26',
          colorBorder: '#000000',
          colorText: '#f2f4f8',
          colorTextSecondary: '#9aa3b2',
          colorBorderSecondary: '#000000',
          fontFamily: "'JetBrains Mono', 'Consolas', 'Segoe UI', system-ui, sans-serif",
        },
        components: {
          Menu: {
            itemBg: 'transparent',
            itemSelectedBg: '#2f6bff',
            itemSelectedColor: '#ffffff',
            itemHoverBg: '#ffffff14',
            itemBorderRadius: 0,
          },
          Card: { headerBg: 'transparent' },
          Button: { fontWeight: 700 },
          Tag: { borderRadiusSM: 0, borderRadiusLG: 0 },
        },
      }}
    >
      <Layout style={{ height: '100vh', overflow: 'hidden', background: '#0a0c10' }}>
        <Sider
          width={248}
          theme="dark"
          style={{
            borderRight: '3px solid #000000',
            background: '#12151b',
          }}
        >
          <div
            style={{
              padding: '18px 14px 14px',
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              borderBottom: '3px solid #000000',
              marginBottom: 12,
            }}
          >
            <img
              src="/icons/logo-w-64.png"
              alt="Firment"
              style={{
                width: 40,
                height: 40,
                borderRadius: 6,
                boxShadow: '3px 3px 0 #000000',
                objectFit: 'contain',
                background: '#2f6bff',
                padding: 4,
              }}
            />
            <div style={{ lineHeight: 1.1 }}>
              <div style={{ fontSize: 17, fontWeight: 800, letterSpacing: 0.5, color: '#ffffff', textTransform: 'uppercase' }}>
                Firment
              </div>
              <div style={{ fontSize: 10, color: '#9aa3b2', letterSpacing: 1.5 }}>
                FIRMWARE + AGENT
              </div>
            </div>
          </div>
          <SessionSidebar
            sessions={sessions}
            currentId={session?.id ?? null}
            workCwd={workCwd}
            onWorkCwd={setWorkCwd}
            onSelect={handleSelectSession}
            onNew={handleNewSession}
            onDelete={handleDeleteSession}
          />
        </Sider>
        <Layout style={{ background: '#0a0c10' }}>
          <Header
            style={{
              height: 54,
              padding: '0 16px',
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              background: '#12151b',
              borderBottom: '3px solid #000000',
            }}
          >
            <Menu
              mode="horizontal"
              selectedKeys={[view]}
              onClick={(e) => setView(e.key as ViewKey)}
              style={{
                flex: 1,
                background: 'transparent',
                borderBottom: 'none',
                fontSize: 13,
                fontWeight: 700,
              }}
              items={[
                { key: 'chat', icon: <MessageOutlined />, label: 'Chat' },
                { key: 'serial', icon: <UsbOutlined />, label: 'Serial monitor' },
                { key: 'flash', icon: <RocketOutlined />, label: 'Flash / Run' },
                { key: 'settings', icon: <ApiOutlined />, label: 'Settings' },
                { key: 'collab', icon: <TeamOutlined />, label: 'Team' },
              ]}
            />
            {running && (
              <Tag
                color="#2f6bff"
                style={{
                  borderRadius: 0,
                  fontWeight: 700,
                  marginInlineEnd: 0,
                  boxShadow: '2px 2px 0 #000000',
                }}
              >
                ⚡ agent running
              </Tag>
            )}
            {session && (
              <Tag
                style={{
                  borderRadius: 0,
                  marginInlineEnd: 0,
                  border: '2px solid #000000',
                  background: '#facc15',
                  color: '#000000',
                  fontWeight: 700,
                }}
              >
                {session.mode}
              </Tag>
            )}
            <Button
              size="small"
              onClick={() => setView('chat')}
              style={{
                borderRadius: 0,
                border: '2px solid #000000',
                background: '#f2f4f8',
                color: '#000000',
                fontWeight: 700,
                boxShadow: '2px 2px 0 #000000',
              }}
            >
              {session?.provider ?? '—'} / {session?.model ?? '—'}
            </Button>
          </Header>
          <Content
            style={{
              overflow: 'hidden',
              display: 'flex',
              flexDirection: 'column',
              minHeight: 0,
              background: '#0a0c10',
            }}
          >
            {view === 'chat' && (
              <ChatView
                session={session}
                running={running}
                turn={turn}
                onSend={handleSend}
                onCancel={handleCancel}
              />
            )}
            {view === 'settings' && <SettingsView />}
            {view === 'serial' && <SerialView lines={monitorLines} />}
            {view === 'flash' && <FlashView />}
            {view === 'collab' && <CollabView />}
          </Content>
        </Layout>
      </Layout>
      {permReq && <PermissionDialog req={permReq} onClose={() => setPermReq(null)} />}
      {askReq && <AskDialog req={askReq} onClose={() => setAskReq(null)} />}
    </ConfigProvider>
  );
}