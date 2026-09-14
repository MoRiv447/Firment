import { describe, expect, it } from 'vitest';
import { isDiffBody, parseDiff } from '../diff';

/**
 * The diff counts, against the shape the backend actually sends.
 *
 * `crates/firment-tools/src/tools/util.rs` writes `--- path\n+++ path\n` before
 * the hunks, and `ToolCard` used to drop only the first of those two lines. The
 * survivor starts with `+`, so a one-line edit came out as `+2 -1` and the body
 * rendered the file header as an added line. These cases are that diff.
 */

/** A one-line replacement, exactly as `simple_diff` emits it. */
const ONE_LINE_EDIT = [
  '--- src/main.c',
  '+++ src/main.c',
  '@@ -10,3 +10,3 @@',
  ' void setup(void) {',
  '-  clock_init(8);',
  '+  clock_init(16);',
  ' }',
].join('\n');

describe('isDiffBody', () => {
  it('recognises a diff by its hunk header', () => {
    expect(isDiffBody(ONE_LINE_EDIT)).toBe(true);
  });

  it('does not treat arbitrary tool output as a diff', () => {
    expect(isDiffBody('Compiling 42 crates\nFinished dev profile')).toBe(false);
    expect(isDiffBody('')).toBe(false);
  });
});

describe('parseDiff', () => {
  it('counts a one-line edit as one line each way', () => {
    const diff = parseDiff(ONE_LINE_EDIT);
    expect(diff).not.toBeNull();
    expect(diff?.added).toBe(1);
    expect(diff?.removed).toBe(1);
  });

  it('drops the file headers so the body cannot paint them as changes', () => {
    const diff = parseDiff(ONE_LINE_EDIT);
    const texts = diff?.lines.map((line) => line.text);
    expect(texts).not.toContain('+++ src/main.c');
    expect(texts).not.toContain('--- src/main.c');
    expect(texts?.[0]).toBe('@@ -10,3 +10,3 @@');
  });

  it('labels each line with the state its colour comes from', () => {
    const kinds = parseDiff(ONE_LINE_EDIT)?.lines.map((line) => line.kind);
    expect(kinds).toEqual(['hunk', 'context', 'removed', 'added', 'context']);
  });

  it('counts an add-only and a delete-only change correctly', () => {
    const added = parseDiff(['--- a', '+++ b', '@@ -1,1 +1,2 @@', ' keep', '+ new'].join('\n'));
    expect([added?.added, added?.removed]).toEqual([1, 0]);

    const removedOnly = parseDiff(['--- a', '+++ b', '@@ -2,2 +1,1 @@', '- gone', ' keep'].join('\n'));
    expect([removedOnly?.added, removedOnly?.removed]).toEqual([0, 1]);
  });

  it('returns null for output that is not a diff', () => {
    // A `set -x` trace prefixes every line it prints with `+ `, which counted as
    // source the build had added.
    expect(parseDiff('+ cargo build\n+ cargo test\n')).toBeNull();
    // A card with no result yet is not an empty diff; it has no diff to draw.
    expect(parseDiff(null)).toBeNull();
    expect(parseDiff(undefined)).toBeNull();
  });

  it('marks a body it had to cut, and counts only what it kept', () => {
    const header = ['--- a', '+++ b', '@@ -1,600 +1,600 @@'].join('\n');
    const detail = `${header}\n${Array.from({ length: 600 }, (_, i) => `+ line ${i}`).join('\n')}`;
    const diff = parseDiff(detail);
    expect(diff?.truncated).toBe(true);
    // The invariant: `+N` in the header is a count of the lines below it, so a
    // cut body reports the change it shows rather than the change it hid.
    expect(diff?.added).toBe(diff?.lines.filter((line) => line.kind === 'added').length);
    expect(diff!.added).toBeLessThan(600);
    // A line cut in half would still start with `+` and be counted.
    expect(diff!.lines[diff!.lines.length - 1].text).toMatch(/^\+ line \d+$/);
  });
});
