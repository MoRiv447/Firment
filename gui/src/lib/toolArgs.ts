/**
 * A tool call's arguments, written the way a person would say them.
 *
 * Every tool card and collapsed tool row used to print
 * `JSON.stringify(call.arguments)` — so the transcript led with
 * `{"path":"src/foc/current.c"}` for a file read, and with the whole argument
 * object for anything with two keys. That is a debug view of an API payload
 * sitting in the most-read part of the interface, and it is why the cards read
 * as raw output rather than as a record of what the agent did.
 *
 * The rule here is: name the thing that was acted on, and mention the rest only
 * when it is short enough to be worth reading. A path is the answer; `glob` and
 * `limit` are context.
 *
 * Pure and total: an agent can send anything, including a string where an
 * object was expected, and none of it may throw in the middle of a transcript
 * render.
 */

/**
 * Keys that name the subject of a call, most specific first.
 *
 * Ordered because several tools carry more than one: `edit_file` has a path and
 * two text bodies, and the path is the part a reader scans for.
 */
const PRIMARY_KEYS = [
  'path',
  'file',
  'files',
  'url',
  'query',
  'pattern',
  'command',
  'cmd',
  'name',
  'key',
  'id',
];

/** How a value reads once it is out of JSON. */
function renderValue(value: unknown): string {
  if (value === null) return 'null';
  if (value === undefined) return 'undefined';
  if (typeof value === 'string') {
    // Quote only when the bare form would be ambiguous — a path or a word reads
    // better without them, and `pattern "dma start"` needs them.
    return /\s/.test(value) ? JSON.stringify(value) : value;
  }
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return `[${value.map(renderValue).join(', ')}]`;
  if (typeof value === 'object') {
    const keys = Object.keys(value as Record<string, unknown>);
    return keys.length === 0 ? '{}' : `{${keys.length} keys}`;
  }
  return String(value);
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * One line describing `args`, at most `max` characters.
 *
 * A string arriving where an object belongs is passed through unchanged: some
 * backends stream arguments as text, and truncating that is still better than
 * showing nothing.
 */
export function describeArgs(args: unknown, max = 90): string {
  if (args === undefined || args === null) return '';

  if (!isPlainObject(args)) {
    const text = typeof args === 'string' ? args : renderValue(args);
    return text.length > max ? `${text.slice(0, max)}…` : text;
  }

  const entries = Object.entries(args).filter(([, v]) => v !== undefined);
  if (entries.length === 0) return '';

  const primaryKey = PRIMARY_KEYS.find((k) => k in args && args[k] !== undefined);

  // Primary value first, then the remaining keys as `key=value`. The primary
  // keeps its bare form (`src/foc/current.c`, not `path=src/foc/current.c`) so
  // the eye lands on the subject rather than on the field name.
  const parts: string[] = [];
  if (primaryKey) {
    parts.push(renderValue(args[primaryKey]));
    for (const [k, v] of entries) {
      if (k === primaryKey) continue;
      parts.push(`${k}=${renderValue(v)}`);
    }
  } else {
    for (const [k, v] of entries) parts.push(`${k}=${renderValue(v)}`);
  }

  const text = parts.join(' · ');
  return text.length > max ? `${text.slice(0, max)}…` : text;
}
