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
  created_at: number;
  updated_at: number;
  messages: ChatMessage[];
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
  | { type: 'turn_start' }
  | { type: 'text_delta'; text: string }
  | { type: 'tool_start'; name: string; args: unknown; seq: number }
  | { type: 'tool_end'; name: string; ok: boolean; summary: string; seq: number }
  | { type: 'turn_end'; text: string }
  | { type: 'info'; message: string }
  | { type: 'settings'; provider: string | null; model: string | null; thinking: string | null; mode: string | null }
  | { type: 'models'; models: string[] }
  | { type: 'sessions'; sessions: SessionSummaryDto[] }
  | { type: 'session_loaded'; session: SessionDto }
  | { type: 'error'; message: string };

export interface PermissionRequest {
  id: number;
  tool: string;
  args: unknown;
  reason: string;
}

export interface AskRequest {
  id: number;
  question: string;
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

export interface ToolCardState {
  seq: number;
  name: string;
  args: unknown;
  status: 'running' | 'ok' | 'failed';
  summary?: string;
}

export interface RunningTurn {
  text: string;
  tools: Record<number, ToolCardState>;
  startedAt: number;
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