import { useEffect, useReducer, useRef, useState } from 'react';
import {
  Badge,
  Button,
  ConfigProvider,
  Dropdown,
  Layout,
  Menu,
  Popover,
  Space,
  Tag,
  theme,
  Tooltip,
  Typography,
} from 'antd';

const { Text } = Typography;
import {
  ApiOutlined,
  BellOutlined,
  MessageOutlined,
  RocketOutlined,
  ProjectOutlined,
  UsbOutlined,
} from '@ant-design/icons';
import {
  api,
  notifySessionsChanged,
  onAgentEvent,
  onAskExpired,
  onAskRequest,
  onMonitorExited,
  onMonitorOutput,
  onPermissionExpired,
  onPermissionRequest,
  onSessionsChanged,
  requestWorkbenchOpen,
} from './lib/api';
import type {
  AskRequest,
  ContextUsageDto,
  MonitorLine,
  NotificationEntry,
  PermissionRequest,
  SessionDto,
  SessionSummaryDto,
} from './types';
import { AskDialog, PermissionDialog } from './components/Dialogs';
import { ChatView } from './views/ChatView';
import { SessionSidebar } from './views/SessionSidebar';
import { SettingsView } from './views/SettingsView';
import { SerialView } from './views/SerialView';
import { initialTurnState, turnsReducer } from './lib/turnReducer';
import type { TurnMap } from './lib/turnReducer';
import { FlashView } from './views/FlashView';
import { WorkbenchView } from './views/WorkbenchView';

const { Sider, Header, Content } = Layout;

type ViewKey = 'chat' | 'settings' | 'serial' | 'flash' | 'collab';

export default function App() {
  const [sessions, setSessions] = useState<SessionSummaryDto[]>([]);
  const [session, setSession] = useState<SessionDto | null>(null);
  // Per-session turn lifecycle: parallel chats each stream their own turn,
  // keyed by session id. The pure turnReducer is reused per slot (unit-tested
  // in __tests__/turnReducer.test.ts); side effects (transcript refresh)
  // stay in the event handler below.
  const [turnsById, dispatchTurn] = useReducer(turnsReducer, undefined, () => ({} as TurnMap));
  const currentTurnState =
    (session ? turnsById[session.id] : undefined) ?? initialTurnState();
  const { running, turn } = currentTurnState;
  const anyRunning = Object.values(turnsById).some((t) => t.running);
  // Info events (stall / tool-wave timeout / compaction notices) surfaced in
  // the chat they belong to.
  const [infos, setInfos] = useState<{ id: number; sid: string | null; text: string }[]>([]);
  // Rough context usage for the OPEN session (header chip); refreshed when
  // the session changes and after every transcript refresh.
  const [usage, setUsage] = useState<ContextUsageDto | null>(null);
  // While the budget menu is open the ctx tooltip stays hidden — otherwise
  // hovering pops the info box and clicking pops two boxes at once.
  const [ctxMenuOpen, setCtxMenuOpen] = useState(false);
  // While the notification panel is open its hover tooltip stays hidden.
  const [notifOpen, setNotifOpen] = useState(false);
  const [view, setView] = useState<ViewKey>('chat');
  // Notification center: guard alerts + build/verify/flash failures + device
  // offline events, aggregated across sessions. Persisted (last 50) so a
  // restart doesn't drop history; unread = entries newer than lastRead.
  const [notifications, setNotifications] = useState<NotificationEntry[]>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem('notifications') || '[]');
      return Array.isArray(saved) ? saved : [];
    } catch {
      return [];
    }
  });
  const [notifLastRead, setNotifLastRead] = useState<number>(
    () => Number(localStorage.getItem('notif-last-read') || 0),
  );
  const notifUnread = notifications.filter((n) => n.ts > notifLastRead).length;
  const pushNotification = (n: Omit<NotificationEntry, 'ts'>) => {
    setNotifications((prev) => {
      if (prev.some((x) => x.id === n.id)) return prev; // dedupe
      const next = [{ ...n, ts: Date.now() }, ...prev].slice(0, 50);
      try {
        localStorage.setItem('notifications', JSON.stringify(next));
      } catch {
        /* quota — history just won't survive restarts */
      }
      return next;
    });
  };
  // Device liveness: last frame timestamp per node + nodes already reported
  // offline (re-armed when traffic resumes). Feeds the offline checker below.
  const deviceLastSeen = useRef(new Map<string, number>());
  const offlineNotified = useRef(new Set<string>());
  useEffect(() => {
    const timer = setInterval(() => {
      const now = Date.now();
      deviceLastSeen.current.forEach((last, node) => {
        if (now - last > 60_000 && !offlineNotified.current.has(node)) {
          offlineNotified.current.add(node);
          pushNotification({
            id: `offline-${node}-${Math.floor(now / 60_000)}`,
            kind: 'device-offline',
            title: `Node offline: ${node}`,
            body: 'No frames received for over 60s (power loss / dropped WiFi?)',
          });
        }
      });
    }, 15_000);
    return () => clearInterval(timer);
  }, []);
  // Permission requests arrive concurrently (tool waves run in parallel), so
  // they must be queued — a single overwriting state would leave the first
  // request unanswered forever and wedge its tool (and the whole wave).
  const [permQueue, setPermQueue] = useState<PermissionRequest[]>([]);
  // Same for ask_user questions: a tool wave can carry several concurrently.
  const [askQueue, setAskQueue] = useState<AskRequest[]>([]);
  const [monitorLines, setMonitorLines] = useState<Record<string, MonitorLine[]>>({});
  const [workCwd, setWorkCwd] = useState('C:\\');
  // The event listeners are registered once ([] deps), so the closure would
  // otherwise capture the FIRST render's `session` (null) forever. Keep a ref
  // to the latest session so turn_end can refresh the transcript.
  const sessionRef = useRef(session);
  sessionRef.current = session;

  useEffect(() => {
    // Single fetch on mount; the restore below re-checks after its await so
    // a fast user selection is never clobbered.
    void api.listSessions().then(setSessions).catch(console.error);
    void api
      .listSessions()
      .then(async (list) => {
        if (list.length === 0 || sessionRef.current) return;
        const s = await api.loadSession(list[0].id);
        if (!sessionRef.current) setSession(s);
      })
      .catch(console.error);
  }, []);

  // Context usage follows the open session and its message count.
  useEffect(() => {
    if (!session) return;
    void api
      .sessionContextUsage(session.id)
      .then(setUsage)
      .catch(() => setUsage(null));
  }, [session?.id, session?.messages.length, session?.context_budget_chars]);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    unlisteners.push(
      onAgentEvent((e) => {
        // Route to the session that owns the event; fall back to the
        // currently open chat for legacy/unstamped events.
        const sid =
          (e as { session_id?: string | null }).session_id ||
          sessionRef.current?.id ||
          null;
        switch (e.type) {
          case 'turn_start':
            setInfos((prev) => prev.filter((i) => i.sid !== sid));
            dispatchTurn(e);
            break;
          case 'text_delta':
          case 'thinking':
          case 'tool_start':
          case 'tool_end':
            // Pure state transitions live in turnReducer (unit-tested);
            // dispatch keeps App.tsx free of the accumulation logic.
            dispatchTurn(e);
            // Notification center: build/verify/flash failures are
            // project-level events worth surfacing even in another chat.
            if (e.type === 'tool_end' && !e.ok && ['build', 'verify', 'flash'].includes(e.name)) {
              pushNotification({
                id: `fail-${sid}-${e.seq}`,
                kind: `${e.name}-fail`,
                title: `${e.name} 失败${sid ? ` · ${sid.slice(0, 8)}` : ''}`,
                body: e.summary.slice(0, 200),
                sid: sid ?? undefined,
              });
            }
            break;
          case 'error':
            // Error ends the turn WITHOUT a turn_end (start_turn emits only
            // Error on failure). Drop stale dialogs ONLY the ones belonging
            // to the finished chat — parallel sessions' pending dialogs must
            // survive.
            dispatchTurn(e);
            setPermQueue((q) => q.filter((r) => r.session_id !== sid));
            setAskQueue((q) => q.filter((r) => r.session_id !== sid));
            break;
          case 'turn_end':
            dispatchTurn(e);
            setPermQueue((q) => q.filter((r) => r.session_id !== sid));
            setAskQueue((q) => q.filter((r) => r.session_id !== sid));
            if (sid && sid === sessionRef.current?.id) {
              // Clear the streaming turn so the transcript (refreshed below)
              // is the single source of truth — otherwise the same reply
              // shows twice (once in turn.text, once in session.messages).
              void api
                .sessionTranscript(sid)
                .then((s) => {
                  // Re-check AFTER the await: switching chats while this
                  // fetch was in flight must not clobber the newly opened
                  // session (the send box then follows sessionRef.current).
                  if (sessionRef.current?.id === s.id) {
                    setSession(s);
                    // Message count changed → refresh the header usage chip.
                    void api.sessionContextUsage(s.id).then(setUsage).catch(() => {});
                  }
                })
                .catch(console.error);
            }
            // The finished session's sidebar row (preview / updated_at)
            // changed on disk either way.
            notifySessionsChanged();
            break;
          case 'session_loaded':
            setSession(e.session);
            // Any session load lands in the chat (workbench "open" button,
            // startup restore, escalation handler all route here).
            setView('chat');
            break;
          case 'sessions':
            setSessions(e.sessions);
            break;
          case 'device_frame': {
            // Notification center source: guard alerts from any node.
            if (e.kind === 'alert') {
              let parsed: Record<string, unknown> = {};
              try {
                parsed = JSON.parse(e.frame);
              } catch {
                /* raw frame */
              }
              const node = String(parsed.node ?? e.node);
              pushNotification({
                id: `alert-${String(parsed.ts ?? Date.now())}-${node}-${String(parsed.rule ?? '')}`,
                kind: 'guard',
                title: `告警 ${node} · ${String(parsed.sev ?? '?')}`,
                body: String(parsed.summary ?? e.frame).slice(0, 200),
              });
            }
            // Device liveness tracking for offline notifications.
            deviceLastSeen.current.set(e.node, Date.now());
            offlineNotified.current.delete(e.node);
            break;
          }
          case 'info':
            console.info('[firm]', e.message);
            // Surface timeouts/stalls in the chat they belong to (sticky,
            // newest last) — and DEDUPE identical consecutive lines (the
            // mqtt retry loop would otherwise spam the same error).
            setInfos((prev) => {
              const last = prev[prev.length - 1];
              if (last && last.text === e.message && last.sid === sid) {
                return prev;
              }
              return [...prev.slice(-8), { id: Date.now(), sid, text: e.message }];
            });
            break;
          // device_frame / guard_status are consumed by the WorkbenchView's
          // own subscriber (the card is self-contained and stays mounted);
          // App-level aggregation was dead weight.
          default:
            break;
        }
      }),
    );

    unlisteners.push(
      onPermissionRequest((req) => setPermQueue((q) => [...q, req])),
      onAskRequest((req) => setAskQueue((q) => [...q, req])),
      onPermissionExpired((id) => {
        // The backend auto-denied after its timeout: drop the stale dialog,
        // otherwise a later Allow click silently no-ops while the user
        // believes they approved the tool.
        setPermQueue((q) => q.filter((r) => r.id !== id));
      }),
      onAskExpired((id) => {
        setAskQueue((q) => q.filter((r) => r.id !== id));
      }),
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

  // Workbench mutations (branch create, set mainline) happen outside this
  // component's state; the event bus is how their views tell us to re-fetch
  // so sidebar tags (NORMAL/MAINLINE/BRANCH) never go stale.
  useEffect(
    () =>
      onSessionsChanged(() => {
        void api.listSessions().then(setSessions).catch(console.error);
      }),
    [],
  );

  // Guard escalation → mainline-session diagnosis. The workbench card
  // synthesizes the prompt; here we load the session, switch to chat and
  // start the turn (refusing politely if that chat is already streaming).
  // Registered ONCE: the running map is read through a ref, so streaming
  // updates don't tear down/re-add the listener.
  const turnsRef = useRef(turnsById);
  turnsRef.current = turnsById;
  useEffect(() => {
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as {
        sessionId: string;
        prompt: string;
      };
      if (turnsRef.current[detail.sessionId]?.running) {
        setInfos((prev) => [
          ...prev.slice(-8),
          {
            id: Date.now(),
            sid: detail.sessionId,
            text: 'escalation skipped: mainline chat already has a turn running',
          },
        ]);
        return;
      }
      void api
        .loadSession(detail.sessionId)
        .then((s) => {
          setSession(s);
          setView('chat');
          void api.startTurn(detail.sessionId, detail.prompt).catch((err) => {
            console.error(err);
            setInfos((prev) => [
              ...prev.slice(-8),
              {
                id: Date.now(),
                sid: detail.sessionId,
                text: `escalation turn failed: ${err}`,
              },
            ]);
          });
        })
        .catch(console.error);
    };
    window.addEventListener('firment:run-escalation', handler);
    return () => window.removeEventListener('firment:run-escalation', handler);
  }, []);

  const handleNewSession = (mode: 'agent' | 'plan') => {
    // Creating a chat never disturbs turns running in other chats.
    void api
      .newSession(workCwd || 'C:\\', mode)
      .then((s) => {
        setSession(s);
        setView('chat');
        void api.listSessions().then(setSessions);
      })
      .catch(console.error);
  };

  const handleSelectSession = (id: string) => {
    // Switching is free: the other chat's turn keeps running in the
    // background and its sidebar row shows a ⚡ badge until it finishes.
    void api
      .loadSession(id)
      .then((s) => {
        setSession(s);
        setView('chat');
      })
      .catch(console.error);
  };

  const handleDeleteSession = (id: string) => {
    // Bounded retry: the backend refuses while that session's turn is
    // winding down, so cancel + retry covers the window — but a hard IO
    // error must not spin forever.
    let attempts = 0;
    const run = () => {
      attempts += 1;
      void api
        .deleteSession(id)
        .then(async () => {
          // Drop the dead session's turn slot (it would otherwise keep its
          // error text/tools alive in memory forever).
          dispatchTurn({ type: 'turn_end', session_id: id, text: '' });
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
        .catch((err: unknown) => {
          console.error(err);
          if (attempts >= 5) {
            setInfos((prev) => [
              ...prev.slice(-8),
              { id: Date.now(), sid: id, text: `delete failed after retries: ${err}` },
            ]);
            return;
          }
          void api.cancelTurn(id).catch(console.error);
          setTimeout(() => void run(), 600);
        });
    };
    if ((turnsById[id] ?? initialTurnState()).running) {
      void api.cancelTurn(id).catch(console.error);
    }
    void run();
  };

  const handleSend = (input: string) => {
    const sid = sessionRef.current?.id;
    if (!sid) return;
    const snapshot = sessionRef.current?.messages ?? [];
    // 乐观追加用户消息，发送后立即显示在聊天区（turn_end 后以 transcript 为准）
    setSession((s) =>
      s ? { ...s, messages: [...s.messages, { role: 'user', content: input }] } : s,
    );
    void api.startTurn(sid, input).catch((err) => {
      console.error(err);
      // 发送失败（如 agent 正忙）：回滚乐观消息，避免界面上出现"幽灵消息"。
      // 只在用户仍停留在同一个会话时回滚——盲目写入快照会把 A 会话的
      // 历史移植到当前打开的 B 会话上。
      setSession((s) => (s && s.id === sid ? { ...s, messages: snapshot } : s));
    });
  };

  const handleCancel = () => {
    if (sessionRef.current) {
      void api.cancelTurn(sessionRef.current.id);
    }
  };

  // Apply a per-session knob change (mode / thinking / budget): the backend
  // persists it and returns the fresh session dto.
  const handleSetSessionProp = (p: Promise<SessionDto>) => {
    void p
      .then((s) => setSession(s))
      .catch((err) => console.error(err));
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
            runningIds={new Set(
              Object.entries(turnsById)
                .filter(([, t]) => t.running)
                .map(([id]) => id),
            )}
            onOpenWorkbench={(projectCwd) => {
              // Hand the project path to the (always-mounted) Workbench view
              // via the event bridge: its localStorage restore only ran once
              // at mount, so just switching tabs shows the WRONG project.
              localStorage.setItem('workbench-last-cwd', projectCwd);
              requestWorkbenchOpen(projectCwd);
              setView('collab');
            }}
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
                { key: 'collab', icon: <ProjectOutlined />, label: 'Workbench' },
              ]}
            />
            {anyRunning && (
              <Tag
                color="#2f6bff"
                style={{
                  borderRadius: 0,
                  fontWeight: 700,
                  marginInlineEnd: 0,
                  boxShadow: '2px 2px 0 #000000',
                }}
              >
                ⚡ {Object.values(turnsById).filter((t) => t.running).length} running
              </Tag>
            )}
            <Popover
              trigger="click"
              placement="bottomRight"
              open={notifOpen}
              onOpenChange={setNotifOpen}
              content={
                <div style={{ width: 380, maxHeight: 420, overflowY: 'auto', padding: 8 }}>
                  <Space style={{ marginBottom: 6, width: '100%', justifyContent: 'space-between' }}>
                    <Button
                      size="small"
                      type="dashed"
                      disabled={notifUnread === 0}
                      onClick={() => {
                        const ts = Date.now();
                        setNotifLastRead(ts);
                        localStorage.setItem('notif-last-read', String(ts));
                      }}
                    >
                      Mark all read
                    </Button>
                    <Button
                      size="small"
                      type="text"
                      onClick={() => {
                        setNotifications([]);
                        localStorage.setItem('notifications', '[]');
                      }}
                    >
                      Clear
                    </Button>
                  </Space>
                  {notifications.length === 0 ? (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      No notifications yet. Guard alerts, build/verify/flash failures and node offline events land here.
                    </Text>
                  ) : (
                    notifications.map((n) => (
                      <div
                        key={n.id}
                        onClick={() => {
                          if (n.sid) {
                            void api.loadSession(n.sid).then((s) => {
                              setSession(s);
                              setView('chat');
                            });
                          }
                        }}
                        style={{
                          padding: '5px 6px',
                          borderBottom: '1px solid #f0f0f0',
                          cursor: n.sid ? 'pointer' : 'default',
                        }}
                      >
                        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                          <Tag
                            color={
                              n.kind === 'guard'
                                ? 'red'
                                : n.kind.includes('fail')
                                  ? 'orange'
                                  : n.kind === 'device-offline'
                                    ? 'default'
                                    : 'blue'
                            }
                            style={{ borderRadius: 0, fontSize: 10, fontWeight: 700 }}
                          >
                            {n.kind}
                          </Tag>
                          <Text style={{ fontSize: 12, flex: 1, fontWeight: 600 }}>{n.title}</Text>
                          <Text type="secondary" style={{ fontSize: 10 }}>
                            {new Date(n.ts).toLocaleTimeString()}
                          </Text>
                        </div>
                        {n.body && (
                          <Text type="secondary" style={{ fontSize: 11, display: 'block' }}>
                            {n.body}
                          </Text>
                        )}
                      </div>
                    ))
                  )}
                </div>
              }
            >
              <Tooltip
                open={notifOpen ? false : undefined}
                title="Notification center (guard alerts / build & flash failures / node offline)"
              >
                <Badge count={notifUnread} size="small" offset={[-2, 2]}>
                  <Button
                    icon={<BellOutlined />}
                    style={{
                      borderRadius: 0,
                      border: '2px solid #000000',
                      boxShadow: '2px 2px 0 #000000',
                    }}
                  />
                </Badge>
              </Tooltip>
            </Popover>
            {session && (
              <Dropdown
                menu={{
                  items: [
                    { key: 'agent', label: 'agent (full tools)' },
                    { key: 'plan', label: 'plan (read-only tools)' },
                  ],
                  onClick: ({ key }) => void handleSetSessionProp(api.setSessionMode(session.id, key)),
                  selectedKeys: [session.mode],
                }}
                trigger={['click']}
                disabled={running}
              >
                <Tag
                  style={{
                    borderRadius: 0,
                    marginInlineEnd: 0,
                    border: '2px solid #000000',
                    background: '#facc15',
                    color: '#000000',
                    fontWeight: 700,
                    cursor: 'pointer',
                  }}
                >
                  {session.mode} ▾
                </Tag>
              </Dropdown>
            )}
            {session && (
              <Dropdown
                menu={{
                  items: [
                    { key: 'off', label: 'thinking: off' },
                    { key: 'low', label: 'thinking: low' },
                    { key: 'medium', label: 'thinking: medium' },
                    { key: 'high', label: 'thinking: high' },
                    { key: 'xhigh', label: 'thinking: xhigh' },
                    { key: 'max', label: 'thinking: max' },
                  ],
                  onClick: ({ key }) => void handleSetSessionProp(api.setSessionThinking(session.id, key)),
                  selectedKeys: [session.thinking],
                }}
                trigger={['click']}
                disabled={running}
              >
                <Tag
                  color="purple"
                  style={{
                    borderRadius: 0,
                    marginInlineEnd: 0,
                    border: '2px solid #000000',
                    fontWeight: 700,
                    cursor: 'pointer',
                  }}
                >
                  🧠 {session.thinking} ▾
                </Tag>
              </Dropdown>
            )}
            {session && (
              <Dropdown
                menu={{
                  items: [
                    { key: '65536', label: '64k chars' },
                    { key: '131072', label: '128k chars' },
                    { key: '262144', label: '256k chars (default)' },
                    { key: '524288', label: '512k chars' },
                    { key: '1048576', label: '1M chars' },
                  ],
                  onClick: ({ key }) =>
                    void handleSetSessionProp(api.setSessionBudget(session.id, Number(key))),
                  selectedKeys: [String(session.context_budget_chars || 262144)],
                }}
                trigger={['click']}
                onOpenChange={setCtxMenuOpen}
                disabled={running}
              >
                <Tooltip
                  open={ctxMenuOpen ? false : undefined}
                  title={
                    usage
                      ? `context ~${Math.round(usage.total_chars / 1024)}k of ${Math.round(usage.budget / 1024)}k chars (${usage.pct.toFixed(0)}%) — click chip to change budget`
                      : 'context usage'
                  }
                >
                  <Tag
                    color={
                      (usage?.pct ?? 0) > 90 ? 'red' : (usage?.pct ?? 0) > 70 ? 'orange' : 'green'
                    }
                    style={{
                      borderRadius: 0,
                      marginInlineEnd: 0,
                      border: '2px solid #000000',
                      fontWeight: 700,
                      cursor: 'pointer',
                    }}
                  >
                    ctx {usage ? `${usage.pct.toFixed(0)}%` : '…'} ▾
                  </Tag>
                </Tooltip>
              </Dropdown>
            )}
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
                infos={infos.filter(
                  (i) => !i.sid || i.sid === session?.id,
                )}
                onSend={handleSend}
                onCancel={handleCancel}
              />
            )}
            {view === 'settings' && <SettingsView />}
            {view === 'serial' && <SerialView lines={monitorLines} />}
            {view === 'flash' && <FlashView />}
            {/* Workbench stays MOUNTED on every tab (hidden, not unmounted):
                its escalation detection + device subscription must keep
                running while the user is in Chat/Serial/etc. */}
            <div
              style={{
                display: view === 'collab' ? 'flex' : 'none',
                flexDirection: 'column',
                flex: 1,
                minHeight: 0,
              }}
            >
              <WorkbenchView />
            </div>
          </Content>
        </Layout>
      </Layout>
      {permQueue[0] && (
        <PermissionDialog
          req={permQueue[0]}
          onClose={() => setPermQueue((q) => q.slice(1))}
        />
      )}
      {askQueue[0] && (
        <AskDialog req={askQueue[0]} onClose={() => setAskQueue((q) => q.slice(1))} />
      )}
    </ConfigProvider>
  );
}