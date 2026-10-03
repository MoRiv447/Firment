import { describe, expect, it } from 'vitest';

/**
 * The wire between the two halves of the app, checked from both ends.
 *
 * `FrontendEvent` in `src-tauri/src/events.rs` and the `FrontendEvent` union in `types.ts` are
 * one contract written twice, and the drift between them does not show up in review: a variant
 * added on one side serialises under a `type` the other has never heard of. A TS consumer cannot
 * narrow to a case that is not in its union, and the App-level switch forwards everything it
 * does not name to the turn reducer — so the ordinary outcome is an event that arrives and is
 * quietly dropped. That is what `settings`, `models` and `sessions` were: three wire kinds with
 * no producer in this application (their `AgentEvent`s come from the TUI's slash commands) and,
 * on the far side, a `case 'sessions'` that could never run.
 *
 * Rule 1 catches the two lists disagreeing. Rule 2 catches a kind that is on the list and read
 * by nobody. Together they make "we emit it, somebody handles it" a checked fact rather than a
 * hope.
 *
 * Sources are read through Vite's `import.meta.glob` with `?raw`, not `node:fs`: this project
 * has no `@types/node` (see the note in `no-literal-tokens.test.ts`) and a test is not a good
 * reason to add one.
 */

const SOURCES = import.meta.glob(
  ['../../**/*.{ts,tsx}', '../../../src-tauri/src/events.rs'],
  { query: '?raw', import: 'default', eager: true },
) as Record<string, string>;

const one = (name: string): string => {
  const found = Object.entries(SOURCES).find(([path]) => path.endsWith(`/${name}`));
  if (!found) throw new Error(`no source read for ${name} — read ${Object.keys(SOURCES).length}`);
  return found[1];
};

/**
 * Kinds in the TS union that never cross the wire: App dispatches them into the turn reducer
 * itself (`turn_synced` is how a chat refetches its transcript after a turn ends). Listing them
 * is what makes the comparison below mean "every event the backend can send is understood here"
 * rather than "the two lists are identical", which they are not by design.
 */
const INTERNAL_KINDS = ['turn_synced'];

/** `TurnStart` and `SubagentEnd` → `turn_start`, `subagent_end`: serde's `rename_all`. */
const snakeCase = (name: string): string =>
  name.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();

const rustKinds = (): string[] => {
  const text = one('events.rs');
  const after = text.slice(text.indexOf('pub enum FrontendEvent'));
  // Stop at the enum's own closing brace: two DTO structs below it are written the same way
  // (`Name {` at four spaces) and are not wire kinds.
  const body = after.slice(0, after.indexOf('\n}'));
  const variants = [...body.matchAll(/^ {4}([A-Z][A-Za-z0-9]*)\s*\{/gm)].map((m) =>
    snakeCase(m[1]),
  );
  expect(variants.length, 'the Rust enum was parsed as empty; the shape changed').toBeGreaterThan(
    5,
  );
  return variants;
};

const tsKinds = (): string[] => {
  const text = one('types.ts');
  const block = text.slice(
    text.indexOf('export type FrontendEvent'),
    text.indexOf('export const TURN_FLOW_KINDS'),
  );
  const kinds = [...block.matchAll(/type: '([a-z_]+)'/g)].map((m) => m[1]);
  expect(kinds.length, 'the TS union was parsed as empty; the shape changed').toBeGreaterThan(5);
  return kinds;
};

/** Files that actually read the bus: the shell's switch, the turn reducer, the traffic hook. */
const READERS = ['App.tsx', 'turnReducer.ts', 'useDeviceTraffic.ts'];

const readBy = (kind: string): boolean =>
  READERS.some((suffix) => {
    const text = one(suffix);
    return text.includes(`case '${kind}'`) || text.includes(`case "${kind}"`);
  });

describe('the agent-event wire', () => {
  it('names the same kinds on both sides of the boundary', () => {
    const rust = new Set(rustKinds());
    const ts = new Set(tsKinds());
    expect(
      INTERNAL_KINDS.filter((kind) => !ts.has(kind)),
      'a kind listed as internal is no longer in the union; take it out of the list',
    ).toEqual([]);
    const tsWire = [...ts].filter((kind) => !INTERNAL_KINDS.includes(kind));
    expect(
      [...rust].filter((kind) => !ts.has(kind)),
      'the backend can send these and TypeScript cannot name them',
    ).toEqual([]);
    expect(
      tsWire.filter((kind) => !rust.has(kind)),
      'the frontend names these but nothing on the wire carries them',
    ).toEqual([]);
  });

  it('is read by something for every kind it can carry', () => {
    const unread = tsKinds().filter((kind) => !readBy(kind));
    expect(
      unread,
      `these wire kinds arrive and nothing switches on them: ${unread.join(', ')} — either read ` +
        'one in App.tsx/turnReducer/useDeviceTraffic, or take it out of both contracts',
    ).toEqual([]);
  });
});
