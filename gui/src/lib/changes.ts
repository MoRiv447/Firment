import type { ChatMessage } from '../types';
import { parseDiff } from './diff';
import { pairAll } from './transcript';

/**
 * Which files the agent changed, and how far they moved.
 *
 * The kernel gives the GUI exactly two things to work from: the call's arguments
 * and whatever the tool printed back. Of the whole tool registry, only
 * `edit_file` and `write_file` print a diff (`crates/firment-core/src/agent.rs`
 * `is_diff_tool`), so this is the honest boundary of what the pane can claim:
 *
 * * a `shell` call that rewrites a file with `sed -i` is invisible here, and
 * * a brand-new file has no diff at all -- the kernel leaves one out on purpose,
 *   because a few hundred `+` lines for a file that did not exist is noise. It is
 *   still a change, so the row says "new file" and carries no counts rather than
 *   being dropped or reported as `+0 -0`.
 *
 * Everything below is derived, never remembered: the transcript is the record, and
 * a pane that kept its own list would be a second list that can disagree with it.
 */

/** The keys that name the file a call acted on, most specific first. */
const PATH_KEYS = ['path', 'file_path', 'file'];

/** What `write_file` prints on success (`crates/firment-tools/src/tools/write_file.rs`). */
const WROTE = /^Wrote \d+ bytes to /;

/** One tool call, in whichever shape the caller has it: a live card, or a call
 *  paired with the text that came back. */
export interface ChangeSource {
  name: string;
  args: unknown;
  detail?: string | null;
}

export interface FileChange {
  path: string;
  /** The file was written into existence during this session. */
  created: boolean;
  /** Counts of `diff` -- the last change that produced one. Null when the only
   *  thing the tools printed was a create, because there is nothing counted. */
  added: number | null;
  removed: number | null;
  /** The newest diff for this file. An earlier one is superseded, and two
   *  half-diffs of the same path read as two files. */
  diff: string | null;
  /** `diff` was cut by the parser's budget, so it does not show the whole change. */
  truncated: boolean;
  /** How many calls touched this file, including ones that changed nothing. */
  edits: number;
}

/**
 * The file a call touched, read from its arguments.
 *
 * `args` is `unknown` (it arrives as JSON from the backend), so every step is
 * checked rather than assumed. The card and this pane both need it, which is why
 * it is here and not in either of them: the day they disagree about which path a
 * call named, the transcript and the Changes list point at different files.
 */
export function editedPath(args: unknown): string | undefined {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return undefined;
  const record = args as Record<string, unknown>;
  for (const key of PATH_KEYS) {
    const value = record[key];
    if (typeof value === 'string' && value) return value;
  }
  return undefined;
}

function touches(source: ChangeSource): boolean {
  return source.name === 'edit_file' || source.name === 'write_file';
}

/**
 * The files a list of calls changed, newest touch first.
 *
 * The counts of a row always describe the diff shown in that row. A file touched
 * three times reports the third diff, not the sum of all three: summing would
 * count the same line twice -- once as removed by edit two, once as added by edit
 * three -- and a `+N` nobody can find in the body is the bug this pass exists to
 * remove. `edits` says how many times it was touched, which is the only honest
 * aggregate.
 */
export function collectChanges(sources: ChangeSource[]): FileChange[] {
  const byPath = new Map<string, FileChange & { order: number }>();
  sources.forEach((source, i) => {
    if (!touches(source)) return;
    const path = editedPath(source.args);
    if (!path) return;
    const detail = source.detail ?? '';
    const diff = parseDiff(detail);
    const created = source.name === 'write_file' && diff === null && WROTE.test(detail);
    // A failed call prints an error where the diff would be. Neither a diff nor
    // "Wrote N bytes" means nothing happened to the file, and nothing that
    // happened to no file belongs in this list.
    if (diff === null && !created) return;
    const existing = byPath.get(path);
    if (!existing) {
      byPath.set(path, {
        path,
        created,
        added: diff?.added ?? null,
        removed: diff?.removed ?? null,
        diff: diff ? detail : null,
        truncated: diff?.truncated ?? false,
        edits: 1,
        order: i,
      });
      return;
    }
    existing.edits += 1;
    existing.order = i;
    // A file created and then edited is still a new file. The reverse cannot
    // happen -- nothing edits a path into being.
    if (diff) {
      existing.added = diff.added;
      existing.removed = diff.removed;
      existing.diff = detail;
      existing.truncated = diff.truncated;
    }
  });
  return [...byPath.values()]
    .sort((a, b) => b.order - a.order)
    .map(({ order: _order, ...change }) => change);
}

/** Turns a paired call into the shape `collectChanges` reads. */
function transcriptSources(messages: ChatMessage[]): ChangeSource[] {
  return pairAll(messages).map((entry) => ({
    name: entry.call.name,
    args: entry.call.arguments,
    detail: entry.result ?? null,
  }));
}

/**
 * Every file the session changed, stored transcript and live turn together.
 *
 * The transcript goes first and the cards after it, which is what makes the
 * merge correct rather than merely stable: a file the stored session edited and
 * the running turn edits again keeps the running turn's diff, because that is
 * the state the working tree is in now.
 *
 * `live` is the union of the turn's own cards and every subagent's steps. A
 * nested agent shares the parent's event sink but not its transcript
 * (`crates/firment-core/src/agent.rs`, the `SubagentStart` docs), so its calls
 * land in neither the parent's stored messages nor the parent's card map — they
 * are in `subagents[].steps`, and a pane that left them out would drop an edit
 * the moment it appeared on screen.
 */
export function sessionChanges(messages: ChatMessage[], live: ChangeSource[]): FileChange[] {
  return collectChanges([...transcriptSources(messages), ...live]);
}

export interface ChangeTotals {
  files: number;
  added: number;
  removed: number;
  /** Files whose only record is a create, so the two totals above cannot cover them. */
  uncounted: number;
}

/** The one-line summary over the list.
 *
 *  `uncounted` exists so the line does not silently drop new files: a session
 *  that wrote four files and edited one is not a `+3 -1` session.
 */
export function changeTotals(changes: FileChange[]): ChangeTotals {
  let added = 0;
  let removed = 0;
  let uncounted = 0;
  for (const change of changes) {
    if (change.added === null || change.removed === null) uncounted += 1;
    else {
      added += change.added;
      removed += change.removed;
    }
  }
  return { files: changes.length, added, removed, uncounted };
}

/** A path is long, absolute, and shared; the tail is what identifies it. */
export function shortenPath(path: string, keep = 2): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= keep) return path;
  return `…/${parts.slice(-keep).join('/')}`;
}
