import { useEffect, useReducer, useRef, useState } from 'react';
import { Button, ConfigProvider, Drawer, Dropdown, Layout, theme, Tooltip } from 'antd';
import {
  BulbFilled,
  BulbOutlined,
  ProjectOutlined,
  SettingOutlined,
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
  FrontendEvent,
  MonitorLine,
  NotificationEntry,
  PermissionRequest,
  SessionDto,
  SessionSummaryDto,
  SettingsDto,
} from './types';
import { AskDialog, PermissionDialog } from './components/Dialogs';
import { ChatView } from './views/ChatView';
import { SessionSidebar } from './views/SessionSidebar';
import { SettingsView } from './views/SettingsView';
import { initialTurnState, turnsReducer } from './lib/turnReducer';
import type { TurnMap } from './lib/turnReducer';
import { WorkbenchView } from './views/WorkbenchView';
import { NotificationBell } from './shell/NotificationBell';
import { Inspector } from './shell/Inspector';
import { StatusBar, StatusDivider, StatusItem } from './shell/StatusBar';
import { TitleBar } from './shell/TitleBar';
import { HardwarePane } from './shell/panes/HardwarePane';
import { PendingPane } from './shell/panes/PendingPane';
import { antdTheme, color, font, setActivePalette } from './styles/tokens';
import {
  ThemeModeContext,
  resolveTheme,
  setThemeSetting,
  useSystemPrefersDark,
  useThemeSetting,
} from './lib/theme';


export default function App() {
  // The colour scheme. `ui.theme` (auto/light/dark) lives in config.toml and is
  // read once here; `auto` additionally follows the OS and re-resolves when the
  // OS flips. Set synchronously during render rather than in an effect, so the
  // first paint is already in the right scheme rather than flashing the other.
  const themeSetting = useThemeSetting();
  const systemIsDark = useSystemPrefersDark(themeSetting === 'auto');
  const mode = resolveTheme(themeSetting, systemIsDark);
  setActivePalette(mode);

  // Read the persisted setting once at startup. Reusing `get_settings` rather
  // than adding a command just for this: it is one local IPC call, and a
  // second source of the same value is a second thing that can disagree.
  // `SettingsView` publishes later changes, so this is the only read.
  //
  // The whole DTO is kept, not just `theme`, because the header's scheme toggle
  // writes the setting back through the same `save_settings` the settings form
  // uses -- and that call replaces the whole object, so a theme-only payload
  // would blank every other field.
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  useEffect(() => {
    void api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setThemeSetting(s.theme ?? 'auto');
      })
      .catch((err: unknown) => console.error(err));
  }, []);

  // Pin the opposite of what is on screen right now. `auto` has no icon of its
  // own, so toggling from `auto` deliberately leaves `auto` behind: the user
  // asked for the other scheme, and staying on `auto` would follow the OS
  // straight back. Settings keeps the three-state control for anyone who wants
  // `auto` back.
  const toggleTheme = () => {
    const next = mode === 'dark' ? 'light' : 'dark';
    setThemeSetting(next);
    if (!settings) return;
    const updated = { ...settings, theme: next };
    setSettings(updated);
    void api.saveSettings(updated).catch((err: unknown) => {
      // The scheme is already applied on screen; a failed write means it will
      // not survive a restart. Say so rather than pretending it saved.
      console.error('theme not persisted:', err);
    });
  };

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
  // The chat the user is looking at. When it changes, a finished turn kept by
  // that chat's slot is superseded by the transcript now on screen: turn_end
  // only refreshes and syncs the chat that was OPEN, so a chat that finished
  // in the background kept its retained copy — reopening it rendered the same
  // reply twice (transcript + live copy) and leaked the slot for the rest of
  // the app's life. A turn still running keeps its buffer (switching to a
  // ⚡ chat mid-stream must not blind it).
  const openChatId = session?.id ?? null;
  const shownChatRef = useRef<string | null>(openChatId);
  useEffect(() => {
    if (shownChatRef.current === openChatId) return;
    shownChatRef.current = openChatId;
    const slot = openChatId ? turnsById[openChatId] : undefined;
    if (slot && !slot.running) {
      dispatchTurn({ type: 'turn_synced', session_id: openChatId });
    }
  }, [openChatId, turnsById]);
  // Info events (stall / tool-wave timeout / compaction notices) surfaced in
  // the chat they belong to. Auto-expire after 15s (sweep below); ids come
  // from a counter — Date.now() collided for same-millisecond entries.
  const [infos, setInfos] = useState<
    { id: number; sid: string | null; text: string; ts: number }[]
  >([]);
  const infoSeqRef = useRef(0);
  const pushInfo = (sid: string | null, text: string) => {
    const id = ++infoSeqRef.current;
    setInfos((prev) => [...prev.slice(-8), { id, sid, text, ts: Date.now() }]);
  };
  // Rough context usage for the OPEN session (header chip); refreshed when
  // the session changes and after every transcript refresh.
  const [usage, setUsage] = useState<ContextUsageDto | null>(null);
  // While the budget menu is open the ctx tooltip stays hidden — otherwise
  // hovering pops the info box and clicking pops two boxes at once.
  // While the notification panel is open its hover tooltip stays hidden.
  const [notifOpen, setNotifOpen] = useState(false);
  // The shell's three panels, replacing a `view` union that drove a five-item
  // tab bar. A tab was the wrong axis: it split one session across five screens,
  // so opening the serial port lost the conversation.
  //
  //   workbenchOpen   a screen you open deliberately, not a tab you pass through
  //   settingsOpen    a drawer, because settings are not a workspace
  //   inspectorOpen   the right column; collapsed by default on a laptop
  const [workbenchOpen, setWorkbenchOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(true);

  // One tick per second while a turn runs, so the status bar's elapsed reading
  // moves. Nothing else in the shell re-renders on it: the value is derived
  // below and passed down as a string.
  const [nowTick, setNowTick] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNowTick(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);
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
    // Reload mid-turn: the reducer starts empty, so a still-running turn
    // would be invisible (no spinner, input re-enabled). Re-light the
    // indicator for every session whose agent still has a live turn; the
    // streamed text/tools return with the next event.
    void api
      .runningSessions()
      .then((ids) => {
        for (const id of ids) dispatchTurn({ type: 'turn_start', session_id: id });
      })
      .catch(console.error);
    // Info banners self-expire: a stall warning that outlives its cause
    // (cleared only on the next turn_start before) sat above the input for
    // the rest of the session's life.
    const sweep = setInterval(() => {
      setInfos((prev) => prev.filter((i) => Date.now() - i.ts < 15_000));
    }, 5_000);
    return () => clearInterval(sweep);
  }, []);

  // Context usage follows the open session and its message count.
  useEffect(() => {
    if (!session) return;
    void api
      .sessionContextUsage(session.id)
      .then(setUsage)
      .catch(() => setUsage(null));
  }, [session?.id, session?.messages.length, session?.context_budget_chars]);

  // A tool-heavy turn can double the context while the chip sits at its
  // turn-start value; poll every 10s while anything is running.
  useEffect(() => {
    if (!anyRunning) return;
    const refresh = setInterval(() => {
      const id = sessionRef.current?.id;
      if (!id) return;
      void api.sessionContextUsage(id).then(setUsage).catch(() => {});
    }, 10_000);
    return () => clearInterval(refresh);
  }, [anyRunning]);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    // Token-burst coalescing: text/thinking deltas accumulate for ~50ms and
    // flush as one batched dispatch, so a fast stream costs ~20 renders/s
    // instead of one full re-render per delta. Non-text events flush the
    // buffer first to preserve ordering.
    const deltaBuffer: FrontendEvent[] = [];
    let deltaTimer: number | null = null;
    const flushDeltas = () => {
      if (deltaTimer !== null) {
        window.clearTimeout(deltaTimer);
        deltaTimer = null;
      }
      const buffered = deltaBuffer.splice(0);
      for (const ev of buffered) dispatchTurn(ev);
    };

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
            deltaBuffer.push(e);
            if (deltaTimer === null) {
              deltaTimer = window.setTimeout(flushDeltas, 50);
            }
            break;
          case 'tool_start':
          case 'tool_end':
            flushDeltas();
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
            // survive. The turn's stale info banners (tool-wave timeout
            // etc.) belong to the same dead turn — clear them too.
            flushDeltas();
            dispatchTurn(e);
            setInfos((prev) => prev.filter((i) => i.sid !== sid));
            setPermQueue((q) => q.filter((r) => r.session_id !== sid));
            setAskQueue((q) => q.filter((r) => r.session_id !== sid));
            break;
          case 'turn_end':
            flushDeltas();
            dispatchTurn(e);
            setPermQueue((q) => q.filter((r) => r.session_id !== sid));
            setAskQueue((q) => q.filter((r) => r.session_id !== sid));
            if (sid && sid === sessionRef.current?.id) {
              // The finished turn stays rendered until the refreshed
              // transcript lands (no blank flash); THIS dispatch drops it in
              // the same React batch as setSession, so the reply is never
              // shown twice.
              void api
                .sessionTranscript(sid)
                .then((s) => {
                  // Re-check AFTER the await: switching chats while this
                  // fetch was in flight must not clobber the newly opened
                  // session (the send box then follows sessionRef.current).
                  if (sessionRef.current?.id !== s.id) return;
                  // A send racing this fetch leaves local state with MORE
                  // messages (optimistic user bubble) than the snapshot —
                  // never roll the visible session back to an older one.
                  const current = sessionRef.current;
                  if (current && current.id === s.id && s.messages.length < current.messages.length)
                    return;
                  setSession(s);
                  // Message count changed → refresh the header usage chip.
                  void api.sessionContextUsage(s.id).then(setUsage).catch(() => {});
                  dispatchTurn({ type: 'turn_synced', session_id: sid });
                })
                .catch(console.error); // fetch failed → keep the live reply
            }
            // The finished session's sidebar row (preview / updated_at)
            // changed on disk either way.
            notifySessionsChanged();
            break;
          case 'session_loaded':
            setSession(e.session);
            // Any session load lands in the chat (workbench "open" button,
            // startup restore, escalation handler all route here).
            setWorkbenchOpen(false);
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
              const id = ++infoSeqRef.current;
              return [...prev.slice(-8), { id, sid, text: e.message, ts: Date.now() }];
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
        pushInfo(
          detail.sessionId,
          'escalation skipped: mainline chat already has a turn running',
        );
        return;
      }
      void api
        .loadSession(detail.sessionId)
        .then((s) => {
          setSession(s);
          setWorkbenchOpen(false);
          void api.startTurn(detail.sessionId, detail.prompt).catch((err) => {
            console.error(err);
            pushInfo(detail.sessionId, `escalation turn failed: ${err}`);
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
        setWorkbenchOpen(false);
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
        setWorkbenchOpen(false);
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
          // error text/tools alive in memory forever). turn_end now RETAINS
          // the finished turn (anti blank-flash), so follow with turn_synced
          // to actually null + prune the slot.
          dispatchTurn({ type: 'turn_end', session_id: id, text: '' });
          dispatchTurn({ type: 'turn_synced', session_id: id });
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
            pushInfo(id, `delete failed after retries: ${err}`);
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
      void api.cancelTurn(sessionRef.current.id).catch(console.error);
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
    // The provider exists for the memoised subtrees: a `React.memo` component
    // compares props only, so without a subscription here a theme flip would
    // leave it painting the previous scheme's colours.
    <ThemeModeContext.Provider value={mode}>
      <ConfigProvider
        theme={{
          ...antdTheme(mode),
          algorithm: mode === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm,
        }}
      >
        <Layout
          style={{
            height: '100vh',
            overflow: 'hidden',
            background: color.bg,
            // Set here as well as in the antd theme: the shell's own text should
            // not depend on a component library token reaching it.
            fontFamily: font.sans,
          }}
        >
          <TitleBar
            project={session?.cwd || workCwd}
            actions={
              <>
                <NotificationBell
                  notifications={notifications}
                  unread={notifUnread}
                  open={notifOpen}
                  onOpenChange={setNotifOpen}
                  onMarkAllRead={() => {
                    const ts = Date.now();
                    setNotifLastRead(ts);
                    localStorage.setItem('notif-last-read', String(ts));
                  }}
                  onClear={() => {
                    setNotifications([]);
                    localStorage.setItem('notifications', '[]');
                  }}
                  onOpenSession={(sid) => {
                    // The notification may outlive its session (deleted while
                    // the panel was open).
                    void api.loadSession(sid).then(setSession).catch(console.error);
                  }}
                />
                <Tooltip title="Project workbench">
                  <Button
                    type="text"
                    aria-label="Project workbench"
                    icon={<ProjectOutlined />}
                    onClick={() => setWorkbenchOpen((o) => !o)}
                    style={workbenchOpen ? { color: color.ink } : undefined}
                  />
                </Tooltip>
                <Tooltip title="Settings">
                  <Button
                    type="text"
                    aria-label="Settings"
                    icon={<SettingOutlined />}
                    onClick={() => setSettingsOpen(true)}
                  />
                </Tooltip>
                <Tooltip
                  title={mode === 'dark' ? 'Switch to the light scheme' : 'Switch to the dark scheme'}
                >
                  <Button
                    type="text"
                    aria-label={mode === 'dark' ? 'Switch to light scheme' : 'Switch to dark scheme'}
                    onClick={toggleTheme}
                    icon={mode === 'dark' ? <BulbOutlined /> : <BulbFilled />}
                  />
                </Tooltip>
              </>
            }
          />

          {/*
            The shell owns its own text colour. antd's `Content` used to supply
            it, and swapping that for a plain `<main>` silently dropped every
            prose colour onto the browser default -- dark text on a dark ground
            for the whole transcript. Same reason `fontFamily` is set on the
            Layout: the shell must not depend on a library token reaching it.
          */}
          <div
            style={{
              flex: 1,
              display: 'flex',
              minHeight: 0,
              minWidth: 0,
              overflow: 'hidden',
              color: color.ink,
            }}
          >
            <aside
              style={{
                width: 248,
                flex: '0 0 auto',
                background: color.surface,
                borderRight: `1px solid ${color.line}`,
                display: 'flex',
                flexDirection: 'column',
                // `minHeight: 0` + `overflow: hidden` are what keep the rail's
                // own footer inside the rail: without them its `marginTop: auto`
                // pushes past the viewport and the version string lands under
                // the status bar.
                minHeight: 0,
                overflow: 'hidden',
              }}
            >
              <SessionSidebar
                sessions={sessions}
                currentId={session?.id ?? null}
                workCwd={workCwd}
                onWorkCwd={setWorkCwd}
                onSelect={handleSelectSession}
                onNew={handleNewSession}
                onDelete={handleDeleteSession}
                runningIds={
                  new Set(
                    Object.entries(turnsById)
                      .filter(([, t]) => t.running)
                      .map(([id]) => id),
                  )
                }
                onOpenWorkbench={(projectCwd) => {
                  setWorkbenchOpen(true);
                  requestWorkbenchOpen(projectCwd);
                }}
              />
            </aside>

            <div style={{ flex: 1, display: 'flex', minWidth: 0, minHeight: 0 }}>
              {/* Both stay mounted and are hidden with `display`. The workbench
                  SUBSCRIBES to the device stream and detects escalations, so
                  unmounting it on every tab switch would blind the guard. */}
              <main
                style={{
                  flex: 1,
                  minWidth: 0,
                  display: workbenchOpen ? 'none' : 'flex',
                  flexDirection: 'column',
                }}
              >
                <ChatView
                  session={session}
                  running={running}
                  turn={turn}
                  infos={infos.filter((i) => !i.sid || i.sid === session?.id)}
                  onSend={handleSend}
                  onCancel={handleCancel}
                />
              </main>
              <main
                style={{
                  flex: 1,
                  minWidth: 0,
                  display: workbenchOpen ? 'flex' : 'none',
                  flexDirection: 'column',
                }}
              >
                <WorkbenchView />
              </main>
            </div>

            <Inspector
              open={inspectorOpen}
              onToggle={() => setInspectorOpen((o) => !o)}
              tabs={[
                {
                  key: 'changes',
                  label: '改动',
                  content: (
                    <PendingPane
                      title="改动卡片"
                      body="每个被改动的文件一张卡：路径、+N −M、hunk、以及改完之后能做什么。一轮改多个文件时先折叠成一张汇总。数据源是 EditJournal，需要一个只读命令把它读出来。"
                    />
                  ),
                },
                {
                  key: 'agents',
                  label: '子代理',
                  content: (
                    <PendingPane
                      title="子代理"
                      body="子代理的步骤和主 agent 共用同一条事件流，但事件上没有深度字段，所以现在分不出是谁干的——它的 read_file 看起来像主 agent 调用的。需要给事件加 depth/agent_id，然后在这里按代理分组显示。"
                    />
                  ),
                },
                {
                  key: 'todos',
                  label: '待办',
                  content: (
                    <PendingPane
                      title="待办"
                      body="agent 的 todo 工具已经在往会话目录写 todos.json（原子写、活过上下文压缩），GUI 一个字都没显示过。读它不需要改内核，只差一个只读命令。"
                    />
                  ),
                },
                {
                  key: 'hardware',
                  label: '硬件',
                  content: <HardwarePane monitorLines={monitorLines} />,
                },
              ]}
            />
          </div>

          <StatusBar>
            <StatusItem
              kind={running ? 'running' : 'neutral'}
              label="轮次"
              value={
                running && turn?.startedAt
                  ? `${Math.max(0, Math.round((nowTick - turn.startedAt) / 1000))}s`
                  : '空闲'
              }
            />
            {anyRunning && (
              <StatusItem
                kind="running"
                value={`${Object.values(turnsById).filter((t) => t.running).length} 个会话在跑`}
              />
            )}
            {session && (
              <>
                <StatusDivider />
                <Dropdown
                  trigger={['click']}
                  disabled={running}
                  menu={{
                    items: [
                      { key: 'agent', label: 'agent（全部工具）' },
                      { key: 'plan', label: 'plan（只读工具）' },
                    ],
                    onClick: ({ key }) => void handleSetSessionProp(api.setSessionMode(session.id, key)),
                  }}
                >
                  <span>
                    <StatusItem
                      kind={session.mode === 'plan' ? 'attention' : 'ok'}
                      label="模式"
                      value={session.mode}
                      title="点击切换 agent / plan"
                    />
                  </span>
                </Dropdown>
                <Dropdown
                  trigger={['click']}
                  disabled={running}
                  menu={{
                    items: ['off', 'low', 'medium', 'high', 'xhigh', 'max'].map((t) => ({
                      key: t,
                      label: `thinking: ${t}`,
                    })),
                    onClick: ({ key }) =>
                      void handleSetSessionProp(api.setSessionThinking(session.id, key)),
                  }}
                >
                  <span>
                    <StatusItem
                      kind="neutral"
                      label="思考"
                      value={session.thinking}
                      title="点击切换思考等级"
                    />
                  </span>
                </Dropdown>
                <Dropdown
                  trigger={['click']}
                  disabled={running}
                  menu={{
                    items: [
                      { key: '65536', label: '64k 字符' },
                      { key: '131072', label: '128k 字符' },
                      { key: '262144', label: '256k 字符（默认）' },
                      { key: '524288', label: '512k 字符' },
                      { key: '1048576', label: '1M 字符' },
                    ],
                    onClick: ({ key }) =>
                      void handleSetSessionProp(api.setSessionBudget(session.id, Number(key))),
                  }}
                >
                  <span>
                    <StatusItem
                      kind={
                        usage === null
                          ? 'neutral'
                          : usage.pct > 90
                            ? 'failed'
                            : usage.pct > 70
                              ? 'attention'
                              : 'ok'
                      }
                      label="上下文"
                      value={usage ? `${usage.pct.toFixed(0)}%` : '…'}
                      title={
                        usage
                          ? `${usage.total_chars} / ${usage.budget} 字符`
                          : '用量未知'
                      }
                    />
                  </span>
                </Dropdown>
                <StatusDivider />
                <StatusItem
                  kind="neutral"
                  value={`${session.provider} · ${session.model}`}
                  title="这个会话使用的 provider 与模型"
                />
              </>
            )}
          </StatusBar>
        </Layout>

        <Drawer
          title="设置"
          placement="right"
          width={760}
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          destroyOnHidden
        >
          <SettingsView />
        </Drawer>
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
    </ThemeModeContext.Provider>
  );
}