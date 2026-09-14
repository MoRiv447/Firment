import { describe, expect, it } from 'vitest';
import { changeTotals, collectChanges, sessionChanges, shortenPath } from '../changes';
import type { ChangeSource } from '../changes';
import type { ChatMessage, ToolCardState } from '../../types';

/**
 * Which files the agent changed, from what the tools printed.
 *
 * The pane summarises a record it does not own, so every rule here is a rule
 * about what that record may be read as: only `edit_file` and `write_file` print
 * diffs, a create prints none, a failed call prints an error where the diff would
 * have been, and a file touched three times has three diffs of which exactly one
 * describes the file as it stands now.
 *
 * `crates/firment-core/src/agent.rs` (`is_diff_tool`) is the source of the tool
 * list, and `crates/firment-tools/src/tools/write_file.rs` the source of the
 * `Wrote N bytes` line. Both are read rather than guessed: a pane that invented a
 * case would report changes that never happened.
 */

/** A one-line replacement, in the shape `simple_diff` writes it. */
const EDIT = [
  '--- src/main.c',
  '+++ src/main.c',
  '@@ -10,3 +10,3 @@',
  ' void setup(void) {',
  '-  clock_init(8);',
  '+  clock_init(16);',
  ' }',
].join('\n');

/** Two added lines, so "the last diff" and "the sum of the diffs" differ. */
const SECOND = [
  '--- src/main.c',
  '+++ src/main.c',
  '@@ -10,3 +10,5 @@',
  ' void setup(void) {',
  '+  spi_init();',
  '+  i2c_init();',
  ' }',
].join('\n');

const edit = (path: string, detail: string | null = EDIT): ChangeSource => ({
  name: 'edit_file',
  args: { path },
  detail,
});

const wrote = (path: string, bytes: number): ChangeSource => ({
  name: 'write_file',
  args: { path },
  detail: `Wrote ${bytes} bytes to ${path}`,
});

/** A live card, as `turnReducer` holds it. */
const card = (name: string, args: unknown, detail?: string): ToolCardState => ({
  seq: 1,
  name,
  args,
  status: 'ok',
  detail,
});

describe('collectChanges', () => {
  it('lists an edited file with its diff and its counts', () => {
    const [change] = collectChanges([edit('src/main.c')]);
    expect(change).toMatchObject({
      path: 'src/main.c',
      added: 1,
      removed: 1,
      created: false,
      edits: 1,
    });
    expect(change.diff).toBe(EDIT);
  });

  it('calls a written-into-existence file a change, with no counts', () => {
    // `write_file` prints a diff only when it overwrote something, so this is the
    // one row that is true without one.
    const [change] = collectChanges([wrote('src/boards/x.c', 512)]);
    expect(change).toMatchObject({ created: true, added: null, removed: null, diff: null });
  });

  it('counts an overwrite, so a rewrite is not a new file', () => {
    const source: ChangeSource = {
      name: 'write_file',
      args: { path: 'src/main.c' },
      detail: `Wrote 90 bytes to src/main.c\n${EDIT}`,
    };
    const [change] = collectChanges([source]);
    expect(change).toMatchObject({ created: false, added: 1, removed: 1 });
  });

  it('leaves out a call that changed nothing', () => {
    // A refused write prints `[Io] write failed: …` where the diff would be. The
    // file is untouched, so listing it would claim an edit nobody made.
    const refused: ChangeSource = {
      name: 'write_file',
      args: { path: 'src/main.c' },
      detail: '[Io] write failed: 拒绝访问。 (os error 5)',
    };
    expect(collectChanges([refused])).toEqual([]);
    // A tool still running has no result at all, which is the same case.
    expect(collectChanges([{ name: 'edit_file', args: { path: 'src/main.c' } }])).toEqual([]);
  });

  it('leaves out tools that do not print a diff, even when their output has one', () => {
    // `shell` output can contain a patch — a `git diff` the agent asked to see.
    // That is a file the agent looked at, not a file it wrote.
    expect(
      collectChanges([{ name: 'shell', args: { command: 'git diff' }, detail: EDIT }]),
    ).toEqual([]);
    expect(
      collectChanges([{ name: 'read_file', args: { path: 'src/main.c' }, detail: EDIT }]),
    ).toEqual([]);
  });

  it('leaves out a call whose arguments name no file', () => {
    expect(collectChanges([{ name: 'edit_file', args: null, detail: EDIT }])).toEqual([]);
    // A backend that streamed arguments as text is not a path to hang a row on.
    expect(collectChanges([{ name: 'edit_file', args: 'src/main.c', detail: EDIT }])).toEqual([]);
  });

  it('reads a path from any of the keys the tools use', () => {
    const sources: ChangeSource[] = [
      { name: 'edit_file', args: { file_path: 'a.c' }, detail: EDIT },
      { name: 'edit_file', args: { file: 'b.c' }, detail: EDIT },
    ];
    expect(collectChanges(sources).map((c) => c.path)).toEqual(['b.c', 'a.c']);
  });

  it('keeps one row per file and the last diff with it', () => {
    // Summing the two would say +3 -1, and nobody could find three added lines in
    // the body under it. `edits` is the only honest aggregate.
    const [change] = collectChanges([edit('src/main.c'), edit('src/main.c', SECOND)]);
    expect(change).toMatchObject({ added: 2, removed: 0, edits: 2 });
    expect(change.diff).toBe(SECOND);
  });

  it('orders by the newest touch, not by the first', () => {
    const paths = collectChanges([
      edit('src/first.c'),
      edit('src/second.c'),
      edit('src/first.c', SECOND),
    ]).map((c) => c.path);
    expect(paths).toEqual(['src/first.c', 'src/second.c']);
  });

  it('says when the diff it is holding was cut short', () => {
    const long = `${EDIT}\n${Array.from({ length: 400 }, (_, i) => `+ line ${i}`).join('\n')}`;
    const [change] = collectChanges([edit('src/main.c', long)]);
    expect(change.truncated).toBe(true);
    // The count counts what is shown, so a cut body never over-claims.
    expect(change.added).toBeLessThan(400);
  });

  it('keeps a file marked new when a later edit gave it a diff', () => {
    const [change] = collectChanges([wrote('src/new.c', 20), edit('src/new.c', SECOND)]);
    expect(change).toMatchObject({ created: true, added: 2, removed: 0, edits: 2 });
  });
});

describe('sessionChanges', () => {
  const callMessage = (id: string, path: string): ChatMessage => ({
    role: 'assistant',
    content: '',
    tool_calls: [{ id, name: 'edit_file', arguments: { path } }],
  });
  const resultMessage = (id: string, content: string): ChatMessage => ({
    role: 'tool',
    tool_call_id: id,
    name: 'edit_file',
    content,
  });

  it('reports the stored transcript, so a reopened session is not blank', () => {
    const changes = sessionChanges(
      [callMessage('t1', 'src/main.c'), resultMessage('t1', EDIT)],
      [],
    );
    expect(changes.map((c) => [c.path, c.added, c.removed])).toEqual([['src/main.c', 1, 1]]);
  });

  it('lets a live card supersede the stored diff for the same file', () => {
    // The working tree holds the newest change, so the newest change is what the
    // row must show — and the row has to move, or the pane would lead with a
    // number nobody can find in the body under it.
    const changes = sessionChanges(
      [callMessage('t1', 'src/main.c'), resultMessage('t1', EDIT)],
      [card('edit_file', { path: 'src/main.c' }, SECOND)],
    );
    expect(changes).toHaveLength(1);
    expect(changes[0]).toMatchObject({ path: 'src/main.c', added: 2, edits: 2 });
  });

  it('carries a subagent step the parent transcript never recorded', () => {
    // `agent.rs` gives a nested agent the parent's event sink and its own session,
    // so its writes are in neither the parent's messages nor the parent's cards.
    const changes = sessionChanges([], [
      card('write_file', { path: 'report.md' }, 'Wrote 4 bytes to report.md'),
    ]);
    expect(changes.map((c) => c.path)).toEqual(['report.md']);
  });

  it('shows nothing for a call that is still running', () => {
    const running: ToolCardState = { ...card('edit_file', { path: 'a.c' }), status: 'running' };
    expect(sessionChanges([], [running])).toEqual([]);
  });
});

describe('changeTotals', () => {
  it('adds the counted rows and names the ones it cannot count', () => {
    const totals = changeTotals(collectChanges([edit('src/a.c'), wrote('src/new.c', 20)]));
    // A session that wrote one file and edited one is not a `+1 -1` session, so
    // the new file is said out loud rather than dropped from the line.
    expect(totals).toEqual({ files: 2, added: 1, removed: 1, uncounted: 1 });
  });

  it('is all zeroes for an empty list', () => {
    expect(changeTotals([])).toEqual({ files: 0, added: 0, removed: 0, uncounted: 0 });
  });
});

describe('shortenPath', () => {
  it('keeps the tail of a Windows absolute path', () => {
    expect(shortenPath('D:\\OldStudy66\\Firment\\src\\foc\\current.c')).toBe('…/foc/current.c');
  });

  it('leaves a path shorter than the keep alone', () => {
    expect(shortenPath('main.c')).toBe('main.c');
    expect(shortenPath('src/main.c', 3)).toBe('src/main.c');
  });
});
