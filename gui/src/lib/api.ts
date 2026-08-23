import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AskRequest,
  ElfCardDto,
  FrontendEvent,
  HardwareExit,
  MonitorLine,
  PermissionRequest,
  QualityItemDto,
  SessionDto,
  SessionSummaryDto,
  SettingsDto,
  TimelineEntryDto,
  WorkbenchStateDto,
} from '../types';

// ---------- session / agent ----------

export const api = {
  startTurn: (input: string) => invoke('start_turn', { input }),
  cancelTurn: () => invoke('cancel_turn'),
  listSessions: () => invoke<SessionSummaryDto[]>('list_sessions'),
  workbenchState: (cwd: string) => invoke<WorkbenchStateDto>('workbench_state', { cwd }),
  workbenchSetMainline: (cwd: string, sessionId: string) =>
    invoke('workbench_set_mainline', { cwd, sessionId }),
  workbenchBranchCreate: (parentId: string, title: string) =>
    invoke<string>('workbench_branch_create', { parentId, title }),
  workbenchElf: (cwd: string, elf?: string) =>
    invoke<ElfCardDto>('workbench_elf', { cwd, elf: elf ?? null }),
  workbenchQuality: (sessionId: string) =>
    invoke<QualityItemDto[]>('workbench_quality', { sessionId }),
  workbenchTimeline: (sessionId: string, limit?: number) =>
    invoke<TimelineEntryDto[]>('workbench_timeline', { sessionId, limit: limit ?? null }),
  newSession: (cwd: string, mode: string) => invoke<SessionDto>('new_session', { cwd, mode }),
  loadSession: (id: string) => invoke<SessionDto>('load_session', { id }),
  deleteSession: (id: string) => invoke('delete_session', { id }),
  sessionTranscript: (id: string) => invoke<SessionDto>('session_transcript', { id }),
  respondPermission: (id: number, allowed: boolean) => invoke('respond_permission', { id, allowed }),
  respondAsk: (id: number, answer: string | null) => invoke('respond_ask', { id, answer }),
  fetchModels: (provider: string) => invoke<string[]>('fetch_models', { provider }),
  setApiKey: (provider: string, key: string) => invoke('set_api_key', { provider, key }),
  getSettings: () => invoke<SettingsDto>('get_settings'),
  saveSettings: (settings: SettingsDto) => invoke('save_settings', { settings }),
  setProvider: (name: string, providerType: string, baseUrl: string | null, model: string) =>
    invoke('set_provider', { name, providerType, baseUrl, model }),
  removeProvider: (name: string) => invoke('remove_provider', { name }),
  // hardware
  listPorts: () => invoke<string[]>('list_ports'),
  monitorStart: (port: string, baud: number, elf: string | null) =>
    invoke('monitor_start', { port, baud, elf }),
  monitorStop: (port: string) => invoke('monitor_stop', { port }),
  monitorSend: (port: string, data: string) => invoke('monitor_send', { port, data }),
  activeMonitors: () => invoke<string[]>('active_monitors'),
  flash: (file: string, chip: string | null, probe: string | null, cwd: string | null) =>
    invoke('flash', { file, chip, probe, cwd }),
  firmRun: (file: string, chip: string | null, probe: string | null, cwd: string | null, timeoutSecs: number) =>
    invoke('firm_run', { file, chip, probe, cwd, timeoutSecs }),
};

// ---------- event wiring ----------

export function onAgentEvent(cb: (e: FrontendEvent) => void): Promise<UnlistenFn> {
  return listen<FrontendEvent>('agent-event', (ev) => cb(ev.payload));
}

export function onPermissionRequest(cb: (e: PermissionRequest) => void): Promise<UnlistenFn> {
  return listen<PermissionRequest>('permission-request', (ev) => cb(ev.payload));
}

export function onAskRequest(cb: (e: AskRequest) => void): Promise<UnlistenFn> {
  return listen<AskRequest>('ask-request', (ev) => cb(ev.payload));
}

export function onMonitorOutput(cb: (e: MonitorLine) => void): Promise<UnlistenFn> {
  return listen<MonitorLine>('monitor-output', (ev) => cb(ev.payload));
}

export function onMonitorExited(cb: (e: { port: string }) => void): Promise<UnlistenFn> {
  return listen<{ port: string }>('monitor-exited', (ev) => cb(ev.payload));
}

export function onHardwareExit(cb: (e: HardwareExit) => void): Promise<UnlistenFn> {
  return listen<HardwareExit>('hardware-exit', (ev) => cb(ev.payload));
}

// ---- cross-view session refresh -------------------------------------------
// The sidebar's session list lives in App; views that mutate sessions
// (Workbench branch/mainline ops, deletes elsewhere) dispatch this window
// event so App re-fetches the list and every tag stays truthful.
const SESSIONS_CHANGED = 'firment:sessions-changed';

export function notifySessionsChanged(): void {
  window.dispatchEvent(new CustomEvent(SESSIONS_CHANGED));
}

export function onSessionsChanged(cb: () => void): () => void {
  const handler = () => cb();
  window.addEventListener(SESSIONS_CHANGED, handler);
  return () => window.removeEventListener(SESSIONS_CHANGED, handler);
}