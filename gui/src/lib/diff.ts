/**
 * The unified diff a tool produced, read once instead of re-parsing it per view.
 *
 * The backend builds `detail` in `crates/firment-tools/src/tools/util.rs`, and its
 * first two lines are file headers:
 *
 *     --- src/main.c
 *     +++ src/main.c
 *     @@ -12,7 +12,9 @@
 *
 * Counting those two lines is the bug this file exists to kill. The card used to
 * drop only the FIRST line -- on the written-down belief that line was an "Edited
 * <path>" summary -- so `+++ src/main.c` survived, was painted as an added line,
 * and every diff on screen reported one change more than the tool had made. The
 * TUI counts the whole body and reports one *removed* line too many for the same
 * reason, so "the same number as the TUI" was never a target worth hitting; the
 * number worth hitting is the one the diff actually contains.
 *
 * A line of real content that happens to begin with `--` or `++` is therefore also
 * dropped, exactly as `diff_is_small` in the TUI drops it. That is a deliberate
 * trade, not an oversight: the alternative is a per-position rule that would
 * miscount the far more common case, and the hunk header already says how many
 * lines the hunk holds.
 */

/** How much of a diff body the card renders before it stops. */
export const DIFF_MAX_CHARS = 4000;

export type DiffLineKind = 'added' | 'removed' | 'context' | 'hunk';

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

export interface Diff {
  lines: DiffLine[];
  added: number;
  removed: number;
  /** The body was cut at `DIFF_MAX_CHARS`, so what is below is not all of it. */
  truncated: boolean;
}

/** A line is a file header when it is a diff `---`/`+++` rather than changed text. */
function isFileHeader(line: string): boolean {
  return line.startsWith('--- ') || line.startsWith('+++ ');
}

/**
 * Whether a blob of tool output is a diff at all.
 *
 * The same predicate the TUI uses (`is_diff_body` in `crates/firment-tui/src/util.rs`):
 * a hunk header. Shell output is the thing this guards against -- a `set -x` trace
 * writes `+ command` for every line it runs, and without this check a build card
 * would claim the trace had added forty lines of source.
 */
export function isDiffBody(text: string): boolean {
  return text.split('\n').some((line) => line.startsWith('@@ '));
}

/**
 * The diff, split into lines that carry their own colour and a count that agrees
 * with them.
 *
 * Returns `null` for anything that is not a diff, so a caller can fall back to
 * "raw output" instead of rendering a body that has no added or removed lines.
 */
export function parseDiff(detail: string | null | undefined): Diff | null {
  if (!detail || !isDiffBody(detail)) return null;

  const lines: DiffLine[] = [];
  let added = 0;
  let removed = 0;
  // Line by line rather than a slice of the text: cutting the string at the cap
  // leaves half of a line, and half a `+` line is both a colour that lies about
  // the edit and a count of something nobody on screen can read.
  let budget = DIFF_MAX_CHARS;
  let truncated = false;
  for (const line of detail.split('\n')) {
    budget -= line.length + 1;
    if (budget < 0) {
      truncated = true;
      break;
    }
    if (isFileHeader(line)) continue;
    let kind: DiffLineKind;
    if (line.startsWith('@@ ')) kind = 'hunk';
    else if (line.startsWith('+')) {
      kind = 'added';
      added += 1;
    } else if (line.startsWith('-')) {
      kind = 'removed';
      removed += 1;
    } else kind = 'context';
    lines.push({ kind, text: line });
  }

  return { lines, added, removed, truncated };
}
