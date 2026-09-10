import { webSearch, webFetch } from './web';
import { readFileSync, listDir, globFiles, grepFiles } from './filesystem';
import { ToolSpec } from '../types';
import toolSpecsJson from './specs.json';

export interface ToolResult {
  success: boolean;
  output: string;
  error?: string;
}

interface JsonSchema {
  required?: string[];
  properties?: Record<string, { type?: string }>;
}

const TOOL_SPECS = toolSpecsJson as ToolSpec[];

/**
 * Reject a call whose arguments cannot mean what the model intended, using the
 * tool's own JSON schema. Empty strings count as missing: `grep` with
 * `{"pattern":""}` is valid JSON, but `new RegExp('')` matches every line of
 * every file in the sandbox, which reads as a successful call that returned
 * garbage. Tools that genuinely take no arguments (`list_dir`) declare an
 * empty `required` list and pass through untouched.
 */
export function validateArgs(toolName: string, args: Record<string, any>): string | null {
  const spec = TOOL_SPECS.find(t => t.name === toolName);
  if (!spec) return null; // unknown tool: handled by the dispatch default arm
  const schema: JsonSchema = spec.input_schema || {};
  if (args === null || typeof args !== 'object' || Array.isArray(args)) {
    return `[InvalidInput] ${toolName}: arguments must be a JSON object`;
  }
  for (const key of schema.required || []) {
    const declaredType = schema.properties?.[key]?.type;
    if (!(key in args)) return `[InvalidInput] ${toolName}: missing required argument "${key}"`;
    const value = args[key];
    if (value === undefined || value === null) {
      return `[InvalidInput] ${toolName}: required argument "${key}" is null`;
    }
    if (declaredType === 'string') {
      if (typeof value !== 'string') {
        return `[InvalidInput] ${toolName}: argument "${key}" must be a string, got ${typeof value}`;
      }
      if (value.trim() === '') {
        return `[InvalidInput] ${toolName}: required argument "${key}" must not be empty`;
      }
    } else if (declaredType === 'integer' || declaredType === 'number') {
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return `[InvalidInput] ${toolName}: argument "${key}" must be a number`;
      }
    } else if (declaredType === 'boolean') {
      if (typeof value !== 'boolean') {
        return `[InvalidInput] ${toolName}: argument "${key}" must be a boolean`;
      }
    }
  }
  return null;
}

export async function executeTool(
  toolName: string,
  args: Record<string, any>,
  cwd: string,
  config: any,
  signal?: AbortSignal
): Promise<ToolResult> {
  const invalid = validateArgs(toolName, args);
  if (invalid) return { success: false, output: '', error: invalid };

  try {
    switch (toolName) {
      case 'read_file': {
        const content = await readFileSync(
          cwd,
          args.path,
          args.offset,
          args.limit,
          args.hashlines
        );
        return { success: true, output: content };
      }

      case 'list_dir': {
        // Schema: path, recursive, limit (NOT depth).
        const result = listDir(cwd, args.path, args.recursive ?? false, args.limit ?? 200);
        return { success: true, output: result };
      }

      case 'glob': {
        // Schema: pattern, root, limit, include_hidden.
        const files = globFiles(
          cwd,
          args.pattern,
          args.root,
          args.limit ?? 200,
          args.include_hidden ?? false
        );
        return {
          success: true,
          output: files.length > 0 ? files.join('\n') : 'No files matched',
        };
      }

      case 'grep': {
        // Schema: pattern, path, glob, case_sensitive, include_hidden, max_results.
        const result = grepFiles(cwd, args.pattern, {
          path: args.path,
          glob: args.glob,
          caseSensitive: args.case_sensitive ?? false,
          includeHidden: args.include_hidden ?? false,
          maxResults: args.max_results ?? 100,
        });
        return { success: true, output: result };
      }

      case 'web_search': {
        const results = await webSearch(
          args.query,
          args.max_results,
          config?.tools?.webSearch || 'duckduckgo',
          signal
        );
        return { success: true, output: formatSearchResults(args.query, results) };
      }

      case 'web_fetch': {
        const content = await webFetch(args.url, signal);
        return { success: true, output: content };
      }

      default:
        return {
          success: false,
          output: '',
          error: `[InvalidInput] Unknown tool: ${toolName}`,
        };
    }
  } catch (err: any) {
    return {
      success: false,
      output: '',
      error: `[${err.message || 'Error'}]`,
    };
  }
}

function formatSearchResults(query: string, results: any[]): string {
  const lines = [`web search results for "${query}" (${results.length}):`];
  for (let i = 0; i < results.length; i++) {
    const r = results[i];
    lines.push(`\n${i + 1}. ${r.title}`);
    lines.push(`   ${r.url}`);
    lines.push(`   ${r.snippet}`);
  }
  if (results.length === 0) lines.push('\n(no results)');
  return lines.join('\n');
}
