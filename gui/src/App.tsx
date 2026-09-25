import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { Bot, Diff, ListChecks, Usb } from 'lucide-react';
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
  SettingsDto,
  TodoDto,
  TurnFlowEvent,
} from './types';
import { AskDialog, PermissionDialog } from './components/Dialogs';
import { ChatView } from './views/ChatView';
import { SessionSidebar } from './views/SessionSidebar';
import { SettingsView } from './views/SettingsView';
import { sessionChanges } from './lib/changes';
import { initialTurnState, turnsReducer } from './lib/turnReducer';
import type { TurnMap } from './lib/turnReducer';
import { WorkbenchView } from './views/WorkbenchView';
import { Inspector } from './shell/Inspector';
import { StatusBar, StatusDivider, StatusItem, StatusMenu, StatusTail } from './shell/StatusBar';
import { TitleBar } from './shell/TitleBar';
import { TitleBarActions } from './shell/TitleBarActions';
import { AgentsPane } from './shell/panes/AgentsPane';
import { TodosPane, todoSummary } from './shell/panes/TodosPane';
import { HardwarePane } from './shell/panes/HardwarePane';
import { ChangesPane } from './shell/panes/ChangesPane';
import { Drawer } from './ui';
import styles from './App.module.css';
import {
  publishScheme,
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

  // The write side of the pre-paint contract in index.html. It can be an
  // effect: the DOM attribute only has to match by the time the browser paints
  // the next frame, and the first frame was already decided by the cache that
  // ran in `index.html`.
  useEffect(() => {
    publishScheme(themeSetting, mode);
  }, [themeSetting, mode]);

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
  const { running, turn, subagents } = currentTurnState;
  const anyRunning = Object.values(turnsById).some((t) => t.running);
  // What the agent has written, for the Changes pane. Three sources, because the
  // transcript alone misses the turn still streaming and the streaming turn alone
  // misses everything from before this app session opened the chat. Subagent steps
  // are the third: a nested run shares the parent's events but not its stored
  // messages, so its edits reach here no other way.
  const changes = useMemo(
    () =>
      sessionChanges(
        session?.messages ?? [],
        [...(turn ? Object.values(turn.tools) : []), ...subagents.flatMap((a) => a.steps)],
      ),
    [session, turn, subagents],
  );
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
  // Rough context usage for the OPEN session (status bar reading); refreshed when
  // the session changes and after every transcript refresh.
  const [usage, setUsage] = useState<ContextUsageDto | null>(null);
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
  // The column's width is the one shell measurement worth keeping: a drag that
  // resets on every restart is a drag that was never really set. The Inspector
  // clamps it, so whatever is in here is only ever a starting point.
  const [inspectorWidth, setInspectorWidth] = useState<number>(
    () => Number(localStorage.getItem('inspector-width')) || 320,
  );
  useEffect(() => {
    localStorage.setItem('inspector-width', String(inspectorWidth));
  }, [inspectorWidth]);

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
  // The agent's own todo list. Read from the file the 	odo tool writes, and
  // refetched when that tool reports -- no polling, because the only thing that
  // can change it is the agent running the tool.
  const [todos, setTodos] = useState<TodoDto[]>([]);
  const [todosLoading, setTodosLoading] = useState(false);
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
    const deltaBuffer: TurnFlowEvent[] = [];
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
            // The agent's todo list is a file the `todo` tool rewrites. Nothing
            // else can change it, so the tool reporting is the only refresh
            // trigger this needs -- and polling a file the agent owns would be
            // the wrong shape for it anyway.
            if (e.type === 'tool_end' && e.name === 'todo' && sid) {
              void api
                .sessionTodos(sid)
                .then((t) => setTodos(Array.isArray(t) ? t : []))
                .catch((err: unknown) => console.error(err));
            }
            // Notification center: build/verify/flash failures are
            // project-level events worth surfacing even in another chat.
            if (e.type === 'tool_end' && !e.ok && ['build', 'verify', 'flash'].includes(e.name)) {
              pushNotification({
                id: `fail-${sid}-${e.seq}`,
                kind: `${e.name}-fail`,
                title: `${e.name} failed${sid ? ` · ${sid.slice(0, 8)}` : ''}`,
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
                title: `alert ${node} · ${String(parsed.sev ?? '?')}`,
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
          // The rest of the shell kinds. `guard_status` is read by the workbench
          // card's own subscriber (`views/workbench/useDeviceTraffic.ts`), same
          // as `device_frame` above, so aggregating it here would only duplicate
          // state that is already rendered.
          //
          // `settings` and `models` have no reader anywhere: SettingsView loads
          // both through `api.settings()` / `api.fetchModels()`. They are ignored
          // out loud rather than forwarded to a reducer that would drop them,
          // because an emitted event nobody consumes is a claim about a feature.
          case 'guard_status':
          case 'settings':
          case 'models':
            break;
          // Everything left is a turn kind — the ones `TURN_FLOW_KINDS` names —
          // and this is the only door to the reducer. Classifying a new kind is
          // now something the build enforces: leave it out of the list and it
          // arrives here with no case to run, instead of being swallowed the way
          // `progress`, `review` and `subagent_*` were when they shipped with a
          // working reducer and no caller.
          default:
            dispatchTurn(e);
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

  // Load the todo list when the open session changes. The list is per session
  // (since the work directory stopped being shared), so switching chats must
  // switch lists -- showing the previous session's items under a new chat was
  // the exact confusion the per-session directory fixed.
  useEffect(() => {
    const id = session?.id;
    if (!id) {
      setTodos([]);
      return;
    }
    let cancelled = false;
    setTodosLoading(true);
    void api
      .sessionTodos(id)
      .then((t) => {
        // Guarded at the IPC boundary: the command returns a list, but a render
        // that assumes it is the worst failure mode there is -- 	odos.length`n        // on a null takes the whole shell down to a blank window.
        if (!cancelled) setTodos(Array.isArray(t) ? t : []);
      })
      .catch((err: unknown) => console.error(err))
      .finally(() => {
        if (!cancelled) setTodosLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [session?.id]);
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
      // The rollback above makes the message disappear, which with nothing
      // said reads as "the click did nothing". `startTurn` refuses while the
      // agent is busy, and the in-transcript quick-action buttons can only
      // ever hit that path.
      pushInfo(sid, `Not sent: ${err}`);
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

  const todosDone = todoSummary(todos);

  return (
    <>
        {/*
          Three rows: the title bar, the working surface, the status bar. Every
          measurement that used to be an inline style here is in `App.module.css`,
          and the scheme's colours come from `[data-scheme]` rather than from a JS
          palette -- a `style` written in React opts its whole subtree out of the
          cascade, which is how the transcript ended up unable to follow the theme.
        */}
        <div className={styles.app}>
          <TitleBar
            project={session?.cwd || workCwd}
            actions={
              <TitleBarActions
                mode={mode}
                onToggleTheme={toggleTheme}
                workbenchOpen={workbenchOpen}
                onToggleWorkbench={() => setWorkbenchOpen((o) => !o)}
                onOpenSettings={() => setSettingsOpen(true)}
                notifications={notifications}
                unread={notifUnread}
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
            }
          />

          <div className={styles.body}>
            <aside className={styles.rail}>
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

            <div className={styles.panes}>
              <main
                data-pane="chat"
                data-hidden={workbenchOpen ? 'true' : undefined}
                className={styles.pane}
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
                data-pane="workbench"
                data-hidden={workbenchOpen ? undefined : 'true'}
                className={styles.pane}
              >
                <WorkbenchView />
              </main>
            </div>

            <Inspector
              open={inspectorOpen}
              onToggle={() => setInspectorOpen((o) => !o)}
              width={inspectorWidth}
              onResize={setInspectorWidth}
              tabs={[
                {
                  key: 'changes',
                  label: 'Changes',
                  icon: Diff,
                  badge: changes.length || undefined,
                  content: <ChangesPane changes={changes} />,
                },
                {
                  key: 'agents',
                  label: 'Subagents',
                  icon: Bot,
                  badge: subagents.length || undefined,
                  content: <AgentsPane subagents={subagents} />,
                },
                {
                  key: 'todos',
                  label: 'Todos',
                  icon: ListChecks,
                  badge: todos.length || undefined,
                  content: <TodosPane todos={todos} loading={todosLoading} />,
                },
                {
                  key: 'hardware',
                  label: 'Hardware',
                  icon: Usb,
                  fill: true,
                  content: <HardwarePane monitorLines={monitorLines} />,
                },
              ]}
            />
          </div>

          <StatusBar>
            <StatusItem
              kind={running ? 'running' : 'neutral'}
              label="Turn"
              value={
                running && turn?.startedAt
                  ? `${Math.max(0, Math.round((nowTick - turn.startedAt) / 1000))}s`
                  : 'idle'
              }
            />
            {anyRunning && (
              <StatusItem
                kind="running"
                value={`${Object.values(turnsById).filter((t) => t.running).length} running`}
              />
            )}
            {session && (
              <>
                <StatusDivider />
                <StatusMenu
                  kind={session.mode === 'plan' ? 'attention' : 'ok'}
                  label="Mode"
                  value={session.mode}
                  title="Switch between agent and plan"
                  disabled={running}
                  options={[
                    { key: 'agent', label: 'agent (all tools)' },
                    { key: 'plan', label: 'plan (read-only tools)' },
                  ]}
                  onSelect={(key) =>
                    void handleSetSessionProp(api.setSessionMode(session.id, key))
                  }
                />
                <StatusMenu
                  kind="neutral"
                  label="Thinking"
                  value={session.thinking}
                  title="Change the thinking level"
                  disabled={running}
                  options={['off', 'low', 'medium', 'high', 'xhigh', 'max'].map((t) => ({
                    key: t,
                    label: `thinking: ${t}`,
                  }))}
                  onSelect={(key) =>
                    void handleSetSessionProp(api.setSessionThinking(session.id, key))
                  }
                />
                <StatusMenu
                  kind={
                    usage === null
                      ? 'neutral'
                      : usage.pct > 90
                        ? 'failed'
                        : usage.pct > 70
                          ? 'attention'
                          : 'ok'
                  }
                  label="Context"
                  value={usage ? `${usage.pct.toFixed(0)}%` : '…'}
                  title={usage ? `${usage.total_chars} / ${usage.budget} chars` : 'usage unknown'}
                  disabled={running}
                  options={[
                    { key: '65536', label: '64k chars' },
                    { key: '131072', label: '128k chars' },
                    { key: '262144', label: '256k chars (default)' },
                    { key: '524288', label: '512k chars' },
                    { key: '1048576', label: '1M chars' },
                  ]}
                  onSelect={(key) =>
                    void handleSetSessionProp(api.setSessionBudget(session.id, Number(key)))
                  }
                />
                <StatusDivider />
                <StatusItem
                  kind="neutral"
                  value={`${session.provider} · ${session.model}`}
                  title="Provider and model for this session"
                />
              </>
            )}
            {/*
              Pinned against the right edge and out of the readings' way. The agent
              owns this list, so the bar only reports where it got to; `null` when
              there is no list, because "0/0" would read as a failed task.
            */}
            {todosDone && (
              <StatusTail>
                <StatusItem
                  kind={todos.every((t) => t.done) ? 'ok' : 'neutral'}
                  label="Todos"
                  value={todosDone}
                  title="The agent's own todo list"
                />
              </StatusTail>
            )}
          </StatusBar>
        </div>

        <Drawer
          title="Settings"
          size="lg"
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
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
    </>
  );
}