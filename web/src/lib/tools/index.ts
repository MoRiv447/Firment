import { webSearch, webFetch } from './web';
import { readFileSync, listDir, globFiles, grepFiles } from './filesystem';

export interface ToolResult {
  success: boolean;
  output: string;
  error?: string;
}

export async function executeTool(
  toolName: string,
  args: Record<string, any>,
  cwd: string,
  config: any
): Promise<ToolResult> {
  try {
    switch (toolName) {
      case 'read_file': {
        const content = await readFileSync(cwd, args.path, args.offset, args.limit);
        return { success: true, output: content };
      }

      case 'list_dir': {
        const result = listDir(cwd, args.path, args.depth);
        return { success: true, output: result };
      }

      case 'glob': {
        const files = globFiles(cwd, args.pattern, args.path);
        return {
          success: true,
          output: files.length > 0 ? files.join('\n') : 'No files matched',
        };
      }

      case 'grep': {
        const result = grepFiles(cwd, args.pattern, args.file_pattern, args.max_results);
        return { success: true, output: result };
      }

      case 'web_search': {
        const results = await webSearch(
          args.query,
          args.max_results,
          config?.tools?.webSearch || 'duckduckgo'
        );
        return { success: true, output: formatSearchResults(args.query, results) };
      }

      case 'web_fetch': {
        const content = await webFetch(args.url);
        return { success: true, output: content };
      }

      case 'task': {
        // Sub-agents are not implemented in the web (serverless) build.
        // Report failure honestly so the model falls back to web_search/web_fetch.
        return {
          success: false,
          output: '',
          error:
            'The task (sub-agent) tool is not available in the web build. Use web_search / web_fetch for research instead.',
        };
      }

      case 'todo': {
        return {
          success: false,
          output: '',
          error: 'The todo tool is not available in the web build. Track steps in your reply instead.',
        };
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
