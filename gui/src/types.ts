export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
  name?: string;
}

export interface SessionDto {
  id: string;
  cwd: string;
  provider: string;
  model: string;
  mode: string;
  thinking: string;
  /** Per-session compaction budget in chars; 0 = agent default. */
  context_budget_chars?: number;
  created_at: number;
  updated_at: number;
  messages: ChatMessage[];
}

export interface ContextUsageDto {
  system_chars: number;
  messages_chars: number;
  budget: number;
  total_chars: number;
  pct: number;
}

export interface PinEntryDto {
  pin: string;
  func: string;
  owner: string;
}

export interface BoardPinmapDto {
  board: string;
  pins: PinEntryDto[];
}

export interface DeviceBindingDto {
  node: string;
  role: string;
  note: string;
  /** Optional device_cmd prefix whitelist (empty = allow all, logged). */
  allow: string[];
}

export interface HardwareInfoDto {
  serial_ports: string[];
  probes: string[];
  probe_rs_available: boolean;
  default_chip: string;
}

export interface FlashHistoryDto {
  ts: number;
  chip: string;
  file: string;
  probe: string | null;
  ok: boolean;
  error: string | null;
}

export interface EscalationEntry {
  id: string;
  ts: number;
  node: string;
  sev: string;
  rule: string;
  summary: string;
  payload: string;
}

export interface NotificationEntry {
  id: string;
  ts: number;
  /** guard | build-fail | verify-fail | flash-fail | device-offline */
  kind: string;
  title: string;
  body: string;
  /** Source session for chat-jumping (when applicable). */
  sid?: string;
}

export interface DecisionEntryDto {
  title: string;
  body: string;
  date: string;
}

export interface KbEntryDto {
  /** Whitelist key: AGENTS.md | docs/vendor-index.toml | cheatsheet:<name>.toml */
  key: string;
  exists: boolean;
  content: string;
  /**
   * Save baseline (disk mtime in ms): null when the file does not exist yet.
   * Pass it back to workbenchKbSave so an externally modified file is not
   * silently overwritten.
   */
  mtimeMs: number | null;
}

/** Aggregated per-node view of SBC device traffic (GUI-side rolling). */
export interface DeviceEntry {
  node: string;
  lastKind: string;
  lastFrame: string;
  ts: number;
  count: number;
}

export interface AlertEntry {
  node: string;
  frame: string;
  ts: number;
}

/** One item of a session's agent-owned todo list. Read-only here: the agent
 *  owns the writes, and a UI that could edit it would be a second writer racing
 *  the tool's atomic save. */
export interface TodoDto {
  text: string;
  done: boolean;
}

export interface SessionSummaryDto {
  id: string;
  updated_at: number;
  model: string;
  cwd: string;
  preview: string;
  kind: string; // "main" | "branch"
  parent_session: string | null;
}

export interface GitStatusDto {
  branch: string;
  dirty_files: number;
}

export interface WorkbenchConfigDto {
  project_name: string;
  mainline_session: string;
  /** Guard escalation threshold from [workbench.guard] (warn default). */
  guard_escalate_sev: string;
  toml_raw: string;
}

export interface WorkbenchStateDto {
  config: WorkbenchConfigDto;
  git: GitStatusDto | null;
  root: string;
}

export interface GateThresholdsDto {
  stack_threshold: number;
  flash_threshold_kib: number;
  ram_threshold_kib: number;
  strict: boolean;
}

export interface ElfCardDto {
  file: string;
  flash_bytes: number;
  ram_bytes: number;
  functions: number;
  gate: GateThresholdsDto | null;
}

export interface QualityItemDto {
  tool: string;
  ok: boolean;
  snippet: string;
}

export interface TimelineFileDto {
  path: string;
  old_lines: number;
  new_lines: number;
}

export interface TimelineEntryDto {
  seq: number;
  created_at: number;
  files: TimelineFileDto[];
}

export type FrontendEvent =
  | { type: 'turn_start'; session_id?: string | null }
  | { type: 'text_delta'; session_id?: string | null; text: string }
  | { type: 'thinking'; session_id?: string | null; text: string }
  | { type: 'tool_start'; session_id?: string | null; name: string; args: unknown; seq: number }
  | {
      type: 'tool_end';
      session_id?: string | null;
      name: string;
      ok: boolean;
      summary: string;
      /** Full output for diff-carrying tools (edit_file/write_file); null for
       * everything else and for the cancel/timeout paths. */
      detail?: string | null;
      seq: number;
    }
  | { type: 'turn_end'; session_id?: string | null; text: string }
  /** A self-review of one tool's change finished (plan §4-A). `seq` names the tool. */
  | {
      type: 'review';
      session_id?: string | null;
      seq: number;
      findings: ReviewFinding[];
    }
  // UI-internal: App dispatches this after the post-turn transcript fetch
  // lands, clearing the retained finished turn (anti blank-flash).
  | { type: 'turn_synced'; session_id?: string | null }
  | { type: 'info'; session_id?: string | null; message: string }
  /** A nested agent started, and everything on this session's stream until the
   * matching `subagent_end` belongs to it.
   *
   * The nested agent shares the parent's sink, so without this pair its tool
   * calls arrive stamped with the PARENT's session_id and read as the main
   * agent's work. The reducer keeps a stack: push on start, pop on end, and
   * route in-between events to the top entry instead of the turn. */
  | {
      type: 'subagent_start';
      session_id?: string | null;
      id: string;
      /** A short form of the prompt -- the question it was asked. */
      label: string;
      depth: number;
    }
  /** Sent on the error path too, so a consumer's stack cannot be left
   * unbalanced by a subagent that failed. */
  | { type: 'subagent_end'; session_id?: string | null; id: string; depth: number }
  | { type: 'device_frame'; node: string; kind: string; frame: string }
  | { type: 'guard_status'; frame: string }
  | { type: 'settings'; provider: string | null; model: string | null; thinking: string | null; mode: string | null }
  | { type: 'models'; models: string[] }
  | { type: 'sessions'; sessions: SessionSummaryDto[] }
  | { type: 'session_loaded'; session: SessionDto }
  | { type: 'error'; session_id?: string | null; message: string };

export interface PermissionRequest {
  id: number;
  tool: string;
  args: unknown;
  reason: string;
  /** Which chat's agent is asking (parallel turns). */
  session_id?: string;
}

export interface AskRequest {
  id: number;
  question: string;
  /** Which chat's agent is asking (parallel turns). */
  session_id?: string;
  options: string[];
}

export interface SettingsDto {
  default_provider: string;
  default_model: string;
  auto_approve: string[];
  max_iterations: number;
  context_budget_chars: number;
  build_command: string | null;
  default_chip: string | null;
  monitor_port: string | null;
  monitor_baud: number;
  web_search: string | null;
  thinking: string;
  /** Colour scheme: "auto" (follow the OS, the default) / "light" / "dark".
   * `ui.theme` in config.toml; an unknown or missing value behaves as "auto". */
  theme: string;
  providers: ProviderEntryDto[];
}

export interface ProviderEntryDto {
  name: string;
  type: string;
  base_url: string | null;
  model: string;
  is_default: boolean;
  api_key: string | null;
}

/**
 * One review finding, mirroring `firment_core::review::Finding`.
 *
 * The same shape every capability produces (dependency / hardware / static / self-review), so
 * a card renders a badge from `severity` and a list from the rest without a second vocabulary.
 * `severity` is lowercase because that is how Rust serialises it.
 */
export interface ReviewFinding {
  id: string;
  title: string;
  severity: 'medium' | 'high';
  category: string;
  file?: string | null;
  symbol?: string | null;
  description: string;
  impact?: string | null;
  code?: string | null;
  steps: string[];
  fix?: string | null;
  tags: string[];
}

export interface ToolCardState {
  seq: number;
  name: string;
  args: unknown;
  /**
   * `unknown` is the reopened-session case: the stored transcript records that a
   * tool was called and what came back, but not whether it succeeded -- a denied
   * call and a timed-out one are plain strings in the tool message. Historical
   * cards render with it, so they do not print a check mark over work that may
   * have failed.
   */
  status: 'running' | 'ok' | 'failed' | 'unknown';
  summary?: string;
  /** Unified diff (header line included) for edit/write tools. */
  detail?: string | null;
  /**
   * The self-review's findings about this tool's change, attached by `seq`.
   *
   * Arrives as its own event *after* the tool ends (the review runs a beat later), which is
   * why the card cannot be final at `tool_end`. A card that ignores this event loses the
   * findings silently — the review ran, the reader never learns.
   */
  findings?: ReviewFinding[];
  /** Wall-clock start for the per-tool elapsed label. */
  startedAt?: number;
  /**
   * Wall-clock end, set the moment the status becomes final.
   *
   * Recorded here rather than derived later so a card's duration and its outcome
   * are written in the same update: a duration that arrived separately could
   * disagree with the check mark next to it, which is the failure `lib/steps.ts`
   * exists to avoid at the row level.
   */
  endedAt?: number;
}

export interface RunningTurn {
  text: string;
  /** Live extended-thinking snippet; cleared once real text starts. */
  thinking: string;
  tools: Record<number, ToolCardState>;
  startedAt: number;
  /**
   * The turn ended but the refreshed transcript has not landed yet: the
   * finished reply stays rendered (no blank flash) until `turn_synced`.
   */
  finished?: boolean;
}

export interface MonitorLine {
  port: string;
  kind: 'stdout' | 'stderr';
  line: string;
}

export interface HardwareExit {
  kind: string;
  code: number;
  stdout: string;
  stderr: string;
}