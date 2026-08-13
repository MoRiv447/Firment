import fs from 'fs';
import path from 'path';
import crypto from 'crypto';

/**
 * Sandbox root. File tools may only access files under this directory.
 * It is fixed to the server's working directory — never derived from
 * client input (a client-supplied `cwd` would make the sandbox check
 * meaningless and allow arbitrary file reads).
 */
const SANDBOX_ROOT = path.resolve(process.cwd());

export async function readFileSync(
  cwd: string,
  filePath: string,
  offset?: number,
  limit?: number
): Promise<string> {
  const resolved = resolvePath(filePath);

  if (!fs.existsSync(resolved)) {
    throw new Error(`[NotFound] File not found: ${filePath}`);
  }
  if (!fs.statSync(resolved).isFile()) {
    throw new Error(`[InvalidInput] Not a file: ${filePath}`);
  }

  const content = fs.readFileSync(resolved, 'utf-8');
  const lines = content.split('\n');

  let output = content;
  if (offset !== undefined || limit !== undefined) {
    const start = Math.max(0, offset ?? 0);
    const end = limit !== undefined ? start + limit : lines.length;
    const sliced = lines.slice(start, end);
    output = `--- ${filePath} (lines ${start}..${end}) ---\n${sliced.join('\n')}`;
  }

  const hash = crypto.createHash('sha256').update(content).digest('hex');
  return `${output}\n[file-sha256: ${hash}]`;
}

export function listDir(cwd: string, dirPath: string = '.', depth: number = 3): string {
  const resolved = resolvePath(dirPath);
  if (!fs.existsSync(resolved)) throw new Error(`[NotFound] Directory not found: ${dirPath}`);
  if (!fs.statSync(resolved).isDirectory()) throw new Error(`[InvalidInput] Not a directory`);
  return formatDirListing(resolved, depth);
}

function formatDirListing(dirPath: string, maxDepth: number, currentDepth = 0): string {
  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    const lines: string[] = [];
    for (const entry of entries) {
      if (entry.name.startsWith('.') || entry.name === 'target' || entry.name === 'node_modules') continue;
      const fullPath = path.join(dirPath, entry.name);
      const relPath = path.relative(SANDBOX_ROOT, fullPath);
      if (entry.isDirectory()) {
        lines.push(`📁 ${relPath}/`);
        if (currentDepth < maxDepth) {
          lines.push(formatDirListing(fullPath, maxDepth, currentDepth + 1));
        }
      } else {
        const stats = fs.statSync(fullPath);
        const size =
          stats.size > 1024 * 1024
            ? `${(stats.size / 1024 / 1024).toFixed(1)}MB`
            : `${(stats.size / 1024).toFixed(1)}KB`;
        lines.push(`📄 ${relPath} (${size})`);
      }
    }
    return lines.join('\n');
  } catch (err: any) {
    return `Error: ${err.message}`;
  }
}

/** Recursive walk of the sandbox matching `pattern` (shell-glob syntax). */
export function globFiles(cwd: string, pattern: string, dirPath: string = '.'): string[] {
  const base = resolvePath(dirPath);
  if (!fs.existsSync(base)) return [];
  const matcher = compileGlob(pattern);
  const out: string[] = [];
  // `walk` yields paths relative to the sandbox root; filter and return them as-is.
  walk(base, 0, (rel) => {
    if (matcher(rel)) out.push(rel);
  });
  return out.slice(0, 500);
}

/** Line-based regex search over sandbox files matching `filePattern`. */
export function grepFiles(cwd: string, pattern: string, filePattern?: string, maxResults = 20): string {
  let regex: RegExp;
  try {
    regex = new RegExp(pattern, 'i');
  } catch (err: any) {
    return `Grep error: invalid pattern: ${err.message}`;
  }
  const fileMatcher = compileGlob(filePattern || '*');
  const results: string[] = [];
  try {
    walk(SANDBOX_ROOT, 0, (rel) => {
      if (results.length >= maxResults) return;
      if (!fileMatcher(rel)) return;
      const full = path.join(SANDBOX_ROOT, rel);
      let content: string;
      try {
        if (!fs.statSync(full).isFile()) return;
        content = fs.readFileSync(full, 'utf-8');
      } catch {
        return; // skip unreadable/binary files
      }
      const lines = content.split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (results.length >= maxResults) break;
        if (regex.test(lines[i])) {
          results.push(`${rel}:${i + 1}: ${lines[i].trim()}`);
        }
      }
    });
  } catch (err: any) {
    return `Grep error: ${err.message}`;
  }
  return results.join('\n') || 'No matches found';
}

/**
 * Resolve `filePath` against the sandbox root and reject anything that
 * escapes it. `filePath` is treated as relative to the sandbox root when it
 * is relative; absolute paths are allowed only if they stay inside the root.
 */
function resolvePath(filePath: string): string {
  const resolved = path.resolve(SANDBOX_ROOT, filePath);
  const root = SANDBOX_ROOT + path.sep;
  if (resolved !== SANDBOX_ROOT && !resolved.startsWith(root)) {
    throw new Error('[InvalidInput] Path escapes workspace');
  }
  return resolved;
}

/** Depth-limited walk invoking `cb(relPath)` for every entry under `root`. */
function walk(root: string, depth: number, cb: (rel: string) => void): void {
  if (depth > 24) return; // hard cap: never recurse deeper than 24 levels
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry.name.startsWith('.') || entry.name === 'target' || entry.name === 'node_modules') continue;
    const full = path.join(root, entry.name);
    const rel = path.relative(SANDBOX_ROOT, full);
    if (entry.isDirectory()) {
      walk(full, depth + 1, cb);
    } else if (entry.isFile()) {
      cb(rel);
    }
  }
}

/**
 * Minimal glob matcher supporting `*` (any chars within a segment), `?`
 * (single char) and `**` (any depth). `*` alone does not cross `/`.
 */
function compileGlob(pattern: string): (name: string) => boolean {
  const segments = pattern.split('/').filter(Boolean);
  const regexParts: string[] = [];
  for (const seg of segments) {
    if (seg === '**') {
      regexParts.push('(?:[^/]*(?:/[^/]*)*)');
    } else {
      const escaped = seg
        .replace(/[.+^${}()|[\]\\]/g, '\\$&')
        .replace(/\*/g, '[^/]*')
        .replace(/\?/g, '[^/]');
      regexParts.push(escaped);
    }
  }
  const re = new RegExp(`^${regexParts.join('/')}$`);
  return (name: string) => re.test(name.replace(/\\/g, '/'));
}
