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

/**
 * Mirrors the read_file spec sent to the model: line-number prefixes,
 * 1000-line default cap with a [truncated] hint, optional hashline anchors.
 */
const READ_FILE_DEFAULT_LIMIT = 1000;

export interface GrepOptions {
  path?: string;
  glob?: string;
  caseSensitive?: boolean;
  includeHidden?: boolean;
  maxResults?: number;
}

export async function readFileSync(
  cwd: string,
  filePath: string,
  offset?: number,
  limit?: number,
  hashlines = false
): Promise<string> {
  const resolved = resolvePath(filePath);

  if (!fs.existsSync(resolved)) {
    throw new Error(`[NotFound] File not found: ${filePath}`);
  }
  if (!fs.statSync(resolved).isFile()) {
    throw new Error(`[InvalidInput] Not a file: ${filePath}`);
  }

  const content = fs.readFileSync(resolved, 'utf-8');
  const allLines = content.split('\n');
  const trailingNewline = allLines.length > 1 && allLines[allLines.length - 1] === '';
  if (trailingNewline) allLines.pop();

  const start = Math.max(0, offset ?? 0);
  const capped =
    limit !== undefined ? Math.min(limit, allLines.length - start)
      : Math.min(READ_FILE_DEFAULT_LIMIT, allLines.length - start);
  const end = start + capped;
  const sliced = allLines.slice(start, end);

  let body: string;
  if (hashlines) {
    // [8-hex content hash] anchor per line, as the spec describes.
    body = sliced
      .map((l, i) => `[${crypto.createHash('sha256').update(l).digest('hex').slice(0, 8)}] ${l}`)
      .join('\n');
  } else {
    // 1-based line numbers so edit targets match what the model sees.
    const width = String(end).length;
    body = sliced.map((l, i) => `${String(start + i + 1).padStart(width)} | ${l}`).join('\n');
  }
  if (end < allLines.length) {
    body += `\n[truncated: showing lines ${start + 1}..${end} of ${allLines.length}; use offset=${end} to continue]`;
  }

  const header = hashlines
    ? `--- ${filePath} ---\n`
    : `--- ${filePath} (lines ${start + 1}..${end} of ${allLines.length}) ---\n`;
  const hash = crypto.createHash('sha256').update(content).digest('hex');
  return `${header}${body}\n[file-sha256: ${hash}]`;
}

export function listDir(
  cwd: string,
  dirPath: string = '.',
  recursive = false,
  limit = 200
): string {
  const resolved = resolvePath(dirPath);
  if (!fs.existsSync(resolved)) throw new Error(`[NotFound] Directory not found: ${dirPath}`);
  if (!fs.statSync(resolved).isDirectory()) throw new Error(`[InvalidInput] Not a directory`);
  return formatDirListing(resolved, recursive ? 24 : 1, limit);
}

function formatDirListing(dirPath: string, maxDepth: number, limit: number, currentDepth = 0): string {
  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    const lines: string[] = [];
    for (const entry of entries) {
      if (lines.length >= limit) {
        lines.push(`…[listing truncated at ${limit} entries]`);
        break;
      }
      if (entry.name.startsWith('.') || entry.name === 'target' || entry.name === 'node_modules') continue;
      const fullPath = path.join(dirPath, entry.name);
      const relPath = path.relative(SANDBOX_ROOT, fullPath);
      if (entry.isDirectory()) {
        lines.push(`📁 ${relPath}/`);
        if (currentDepth < maxDepth) {
          lines.push(formatDirListing(fullPath, maxDepth, limit - lines.length, currentDepth + 1));
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

/** Glob file search rooted at `root` inside the sandbox. */
export function globFiles(
  cwd: string,
  pattern: string,
  root: string = '.',
  limit = 200,
  includeHidden = false
): string[] {
  const base = resolvePath(root);
  if (!fs.existsSync(base)) return [];
  const matcher = compileGlob(pattern);
  const out: string[] = [];
  // `walk` yields paths relative to the sandbox root; filter and return them as-is.
  walk(base, 0, includeHidden, (rel) => {
    if (matcher(rel)) out.push(rel);
  });
  return out.slice(0, limit);
}

/** Line-based regex search over sandbox files matching an optional glob. */
export function grepFiles(cwd: string, pattern: string, opts: GrepOptions = {}): string {
  const {
    path: searchPath = '.',
    glob: fileGlob,
    caseSensitive = false,
    includeHidden = false,
    maxResults = 100,
  } = opts;
  let regex: RegExp;
  try {
    regex = new RegExp(pattern, caseSensitive ? '' : 'i');
  } catch (err: any) {
    return `Grep error: invalid pattern: ${err.message}`;
  }
  const base = resolvePath(searchPath || '.');
  const fileMatcher = fileGlob ? compileGlob(fileGlob) : null;
  const results: string[] = [];
  try {
    walk(base, 0, includeHidden, (rel) => {
      if (results.length >= maxResults) return;
      if (fileMatcher && !fileMatcher(rel)) return;
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
 * Hidden segments (.env.local, .next/, .git/…) are rejected outright:
 * directory listings never show them and handing them to the model would
 * leak server secrets on a self-hosted deployment.
 */
function resolvePath(filePath: string): string {
  const segments = filePath.split(/[\\/]+/).filter((s) => s !== '' && s !== '.');
  if (segments.some((s) => s.startsWith('.'))) {
    throw new Error('[Permission] hidden files/directories are not accessible in web mode');
  }
  const resolved = path.resolve(SANDBOX_ROOT, filePath);
  const root = SANDBOX_ROOT + path.sep;
  if (resolved !== SANDBOX_ROOT && !resolved.startsWith(root)) {
    throw new Error('[InvalidInput] Path escapes workspace');
  }
  return resolved;
}

/** Depth-limited walk invoking `cb(relPath)` for every entry under `root`. */
function walk(root: string, depth: number, includeHidden: boolean, cb: (rel: string) => void): void {
  if (depth > 24) return; // hard cap: never recurse deeper than 24 levels
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (
      (!includeHidden && entry.name.startsWith('.')) ||
      entry.name === 'target' ||
      entry.name === 'node_modules'
    ) {
      continue;
    }
    const full = path.join(root, entry.name);
    const rel = path.relative(SANDBOX_ROOT, full);
    if (entry.isDirectory()) {
      walk(full, depth + 1, includeHidden, cb);
    } else if (entry.isFile()) {
      cb(rel);
    }
  }
}

/**
 * Minimal glob matcher supporting `*` (any chars within a segment), `?`
 * (single char) and `**` (any depth — including ZERO directories, like
 * globset: a leading star-star slash still matches root-level files).
 */
function compileGlob(pattern: string): (name: string) => boolean {
  const esc = pattern
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*\*/g, '\u0000')
    .replace(/\*/g, '[^/]*')
    .replace(/\?/g, '[^/]')
    .replace(/\u0000\//g, '(?:[^/]+/)*')
    .replace(/\u0000/g, '.*');
  const re = new RegExp(`^${esc}$`);
  return (name: string) => re.test(name.replace(/\\/g, '/'));
}
