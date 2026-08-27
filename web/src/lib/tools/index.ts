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
          config?.tools?.webSearch || 'duckduckgo'
        );
        return { success: true, output: formatSearchResults(args.query, results) };
      }

      case 'web_fetch': {
        const content = await webFetch(args.url);
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
