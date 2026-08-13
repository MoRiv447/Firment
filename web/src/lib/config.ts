import { ToolSpec } from './types';

export interface ProviderConfig {
  type: 'openai' | 'anthropic';
  baseUrl: string;
  apiKey: string;
  model: string;
  maxTokens?: number;
  temperature?: number;
}

export interface ToolsConfig {
  webSearch: string;
  workspace?: string;
  maxSubagentDepth: number;
}

export interface Config {
  providers: Record<string, ProviderConfig>;
  defaultProvider: string;
  tools: ToolsConfig;
  contextBudgetChars: number;
  maxIterations: number;
  thinking: string;
}

export function getStoredConfig(): Config {
  try {
    const saved = localStorage.getItem('firment-config');
    if (saved) {
      return JSON.parse(saved);
    }
  } catch {}
  return DEFAULT_CONFIG;
}

export function saveConfig(config: Config): void {
  try {
    localStorage.setItem('firment-config', JSON.stringify(config));
  } catch (e) {
    console.error('Failed to save config:', e);
  }
}

export const DEFAULT_CONFIG: Config = {
  providers: {
    default: {
      type: 'openai',
      baseUrl: 'https://api.deepseek.com/v1',
      apiKey: '',
      model: 'deepseek-v4-flash',
    },
  },
  defaultProvider: 'default',
  tools: {
    webSearch: 'bing',
    maxSubagentDepth: 2,
  },
  contextBudgetChars: 60000,
  maxIterations: 30,
  thinking: 'off',
};

// Tool specs come from the core registry (`firm tools`), not a hand-written
// copy — this is the single source of truth. Only the subset the web tool
// executor (tools/index.ts) actually implements is exposed to the model, so
// the model never sees a tool the web surface cannot run.
import toolSpecsJson from './tools/specs.json';

const WEB_TOOL_WHITELIST = new Set([
  'read_file',
  'list_dir',
  'glob',
  'grep',
  'web_search',
  'web_fetch',
]);

export const WEB_TOOL_SPECS: ToolSpec[] = (toolSpecsJson as ToolSpec[]).filter(
  (t) => WEB_TOOL_WHITELIST.has(t.name),
);
