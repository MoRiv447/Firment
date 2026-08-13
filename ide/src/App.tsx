import { useEffect, useReducer, useRef, useState } from 'react';
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
  SessionDto,
  SessionSummaryDto,
} from './types';
import { AskDialog, PermissionDialog } from './components/Dialogs';
import { ChatView } from './views/ChatView';
import { SessionSidebar } from './views/SessionSidebar';
import { SettingsView } from './views/SettingsView';
import { SerialView } from './views/SerialView';
import { initialTurnState, turnReducer } from './lib/turnReducer';
import { FlashView } from './views/FlashView';
import { CollabView } from './views/CollabView';

const { Sider, Header, Content } = Layout;

type ViewKey = 'chat' | 'settings' | 'serial' | 'flash' | 'collab';

export default function App() {
  const [sessions, setSessions] = useState<SessionSummaryDto[]>([]);
  const [session, setSession] = useState<SessionDto | null>(null);
  // Turn lifecycle state is driven by the pure turnReducer (unit-tested in
  // __tests__/turnReducer.test.ts); side effects (transcript refresh) stay
  // in the event handler below.
  const [turnState, dispatchTurn] = useReducer(turnReducer, undefined, initialTurnState);
  const { running, turn } = turnState;
  // Latest `running` value for event-handler closures (registered once with
  // [] deps) without stale-state captures.
  const runningRef = useRef(running);
  runningRef.current = running;
  // A session switch requested while a turn is running: cancel first, then
  // run the pending action when turn_end lands.
  const pendingSwitchRef = useRef<(() => void) | null>(null);
  // Info events (stall / tool-wave timeout / compaction notices) surfaced in
  // the chat so the user can tell a slow model from a wedged turn.
  const [infos, setInfos] = useState<{ id: number; text: string }[]>([]);
  const [view, setView] = useState<ViewKey>('chat');
  // Permission requests arrive concurrently (tool waves run in parallel), so
  // they must be queued — a single overwriting state would leave the first
  // request unanswered forever and wedge its tool (and the whole wave).
  const [permQueue, setPermQueue] = useState<PermissionRequest[]>([]);
  const [askReq, setAskReq] = useState<AskRequest | null>(null);
  const [monitorLines, setMonitorLines] = useState<Record<string, MonitorLine[]>>({});
  const [workCwd, setWorkCwd] = useState('C:\\');
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
            setInfos([]);
            dispatchTurn(e);
            break;
          case 'text_delta':
          case 'tool_start':
          case 'tool_end':
            // Pure state transitions live in turnReducer (unit-tested);
            // dispatch keeps App.tsx free of the accumulation logic.
            dispatchTurn(e);
            break;
          case 'error':
            // Error ends the turn WITHOUT a turn_end (start_turn emits only
            // Error on failure), so the queued session switch must run here
            // too — otherwise switching mid-turn that dies with an error
            // would silently never happen. Also drop stale dialogs.
            dispatchTurn(e);
            setPermQueue([]);
            setAskReq(null);
            runPendingSwitch();
            break;
          case 'turn_end':
            dispatchTurn(e);
            // A session switch queued while the turn was running: the cancel
            // fired from the sidebar handler lands here, so run it now that
            // the turn is truly over (the transcript refresh below would
            // otherwise race the session swap).
            setPermQueue([]);
            setAskReq(null);
            if (runPendingSwitch()) {
              break;
            }
            // Clear the streaming turn so the transcript (refreshed below)
            // is the single source of truth — otherwise the same reply shows
            // twice (once in turn.text, once in session.messages).
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
            // Surface timeouts/stalls in the chat (sticky, newest last) so
            // the user can tell a slow model from a wedged turn.
            setInfos((prev) => [...prev.slice(-4), { id: Date.now(), text: e.message }]);
            break;
          default:
            break;
        }
      }),
    );

    unlisteners.push(
      onPermissionRequest((req) => setPermQueue((q) => [...q, req])),
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

  // Run the session switch queued while a turn was running. Returns true when
  // a switch was pending (the caller must skip its own post-turn work).
  const runPendingSwitch = () => {
    const pending = pendingSwitchRef.current;
    pendingSwitchRef.current = null;
    if (pending) {
      pending();
      return true;
    }
    return false;
  };

  const handleNewSession = async (mode: 'agent' | 'plan') => {
    const run = () => {
      void api
        .newSession(workCwd || 'C:\\', mode)
        .then((s) => {
          setSession(s);
          setView('chat');
          void api.listSessions().then(setSessions);
        })
        .catch(console.error);
    };
    if (runningRef.current) {
      // Cancel the in-flight turn; turn_end fires the queued action.
      pendingSwitchRef.current = run;
      void api.cancelTurn().catch(console.error);
    } else {
      run();
    }
  };

  const handleSelectSession = async (id: string) => {
    const run = () => {
      void api
        .loadSession(id)
        .then((s) => {
          setSession(s);
          setView('chat');
        })
        .catch(console.error);
    };
    if (runningRef.current) {
      pendingSwitchRef.current = run;
      void api.cancelTurn().catch(console.error);
    } else {
      run();
    }
  };

  const handleDeleteSession = async (id: string) => {
    const run = () => {
      void api
        .deleteSession(id)
        .then(async () => {
          const list = await api.listSessions();
          setSessions(list);
          // If the current session was deleted, switch to the newest remaining
          // one (or clear) so the UI never keeps showing a deleted session.
          if (sessionRef.current?.id === id) {
            if (list.length > 0) {
              const s = await api.loadSession(list[0].id);
              setSession(s);
            } else {
              setSession(null);
            }
          }
        })
        .catch(console.error);
    };
    // Deleting while a turn runs would let run_turn's final save resurrect
    // the session — cancel first, delete when the turn actually ends.
    if (runningRef.current && sessionRef.current?.id === id) {
      pendingSwitchRef.current = run;
      void api.cancelTurn().catch(console.error);
    } else {
      run();
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
                infos={infos}
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
      {permQueue[0] && (
        <PermissionDialog
          req={permQueue[0]}
          onClose={() => setPermQueue((q) => q.slice(1))}
        />
      )}
      {askReq && <AskDialog req={askReq} onClose={() => setAskReq(null)} />}
    </ConfigProvider>
  );
}