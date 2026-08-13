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
    webSearch: 'duckduckgo',
    maxSubagentDepth: 2,
  },
  contextBudgetChars: 60000,
  maxIterations: 30,
  thinking: 'off',
};

export const WEB_TOOL_SPECS: ToolSpec[] = [
  {
    name: 'read_file',
    description: 'Read a text file. Optionally slice by line offset and limit.',
    input_schema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'File path, absolute or relative to the workspace' },
        offset: { type: 'integer', minimum: 0, description: '0-based line offset to start reading from' },
        limit: { type: 'integer', minimum: 1, description: 'Maximum number of lines to read' },
      },
      required: ['path'],
    },
  },
  {
    name: 'list_dir',
    description: 'List directory contents with optional depth.',
    input_schema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Directory path, defaults to current directory' },
        depth: { type: 'integer', description: 'Maximum directory depth to traverse', default: 3 },
      },
      required: ['path'],
    },
  },
  {
    name: 'glob',
    description: 'Find files matching a glob pattern.',
    input_schema: {
      type: 'object',
      properties: {
        pattern: { type: 'string', description: 'Glob pattern, e.g. "*.ts" or "**/*.rs"' },
        path: { type: 'string', description: 'Base directory to search in', default: '.' },
      },
      required: ['pattern'],
    },
  },
  {
    name: 'grep',
    description: 'Search for a pattern in files.',
    input_schema: {
      type: 'object',
      properties: {
        pattern: { type: 'string', description: 'Regular expression pattern to search for' },
        file_pattern: { type: 'string', description: 'File pattern to search in, e.g. "*.ts"' },
        max_results: { type: 'integer', description: 'Maximum number of results', default: 20 },
      },
      required: ['pattern'],
    },
  },
  {
    name: 'web_search',
    description: 'Search the web and return the top results (title, URL, snippet).',
    input_schema: {
      type: 'object',
      properties: {
        query: { type: 'string', description: 'The search query' },
        max_results: { type: 'integer', minimum: 1, maximum: 8, default: 5 },
      },
      required: ['query'],
    },
  },
  {
    name: 'web_fetch',
    description: 'Fetch a URL and return its readable text content.',
    input_schema: {
      type: 'object',
      properties: {
        url: { type: 'string', description: 'The URL to fetch' },
      },
      required: ['url'],
    },
  },
];
