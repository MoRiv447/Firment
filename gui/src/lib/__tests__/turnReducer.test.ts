import { describe, expect, it } from 'vitest';
import { initialTurnState, turnReducer, turnsReducer, type TurnState } from '../turnReducer';
import type { FrontendEvent, ReviewFinding } from '../../types';

function feed(events: FrontendEvent[], start: TurnState = initialTurnState()): TurnState {
  return events.reduce(turnReducer, start);
}

const finding: ReviewFinding = {
  id: 'r1',
  title: 'unchecked cast',
  severity: 'high',
  category: 'self-review',
  description: 'the cast discards the length check',
  steps: [],
  tags: [],
};

describe('turnReducer (IDE event->UI contract)', () => {
  it('attaches a phase to the card it names, by seq', () => {
    // The phase means "what it is doing now", so it is joined the same way the review is — by
    // `seq`, the key the card is filed under — and it arrives on its own event rather than with
    // the call. Whether it is *shown* is a separate decision: the two-second rule lives where the
    // card is drawn, because this file is pure and has no clock (see ToolCard).
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'flash', args: {}, seq: 5 },
      { type: 'progress', tool: 'flash', seq: 5, phase: 'downloading', current: 0, total: 0 },
    ]);
    expect(state.turn?.tools[5].progress).toBe('downloading');

    // A later phase replaces the earlier one: the card shows the newest thing, not a history.
    const later = turnReducer(state, {
      type: 'progress',
      tool: 'flash',
      seq: 5,
      phase: 'verifying',
      current: 0,
      total: 0,
    });
    expect(later.turn?.tools[5].progress).toBe('verifying');
  });

  it('drops the phase when the tool ends', () => {
    // "downloading" describes a present tense. Left set, it would sit under the result mark for
    // the rest of the turn — and the renderer's two-second rule cannot help, because the card has
    // long since passed it.
    const done = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'flash', args: {}, seq: 5 },
      { type: 'progress', tool: 'flash', seq: 5, phase: 'downloading', current: 0, total: 0 },
      { type: 'tool_end', name: 'flash', ok: true, summary: 'flash passed', seq: 5 },
    ]);
    expect(done.turn?.tools[5].status).toBe('ok');
    expect(done.turn?.tools[5].progress).toBeUndefined();
  });

  it("routes a nested agent's phase to its step, and drops one with no card", () => {
    // Each agent numbers its own calls, so the turn and the subagent can both hold a `#6`.
    // The phase says which one it belongs to, and only that card changes.
    const nested = feed([
      { type: 'turn_start' },
      { type: 'subagent_start', id: 's1', label: 'research', depth: 1 },
      { type: 'tool_start', name: 'build', args: {}, seq: 6, owner: 's1' },
      { type: 'tool_start', name: 'build', args: {}, seq: 6 },
      {
        type: 'progress',
        tool: 'build',
        seq: 6,
        owner: 's1',
        phase: 'compiling',
        current: 0,
        total: 0,
      },
    ]);
    expect(nested.subagents[0].steps[0].progress).toBe('compiling');
    expect(nested.turn?.tools[6].progress).toBeUndefined();

    const orphan = feed([
      { type: 'turn_start' },
      { type: 'progress', tool: 'ghost', seq: 9, phase: 'nowhere', current: 0, total: 0 },
    ]);
    expect(orphan.turn?.tools[9]).toBeUndefined();
  });

  it('resolves a card that never got its result when the turn ends', () => {
    // Every agent path emits `tool_end`, so a card still running at `turn_end` means the event
    // was lost. Leaving it says "still working" forever; the summary names the absence instead
    // of inventing an outcome.
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'build', args: {}, seq: 7 },
      { type: 'subagent_start', id: 's1', label: 'research', depth: 1 },
      { type: 'tool_start', name: 'grep', args: {}, seq: 1, owner: 's1' },
      { type: 'turn_end', text: 'done' },
    ]);
    expect(state.turn?.tools[7].status).toBe('failed');
    expect(state.turn?.tools[7].summary).toContain('no result reported');
    // The nested list survives as the record of what ran — with its own dangling card closed.
    expect(state.subagents[0].steps[0].status).toBe('failed');

    // A card that did report is left exactly as it was.
    const reported = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'build', args: {}, seq: 8 },
      { type: 'tool_end', name: 'build', ok: true, summary: 'exit 0', seq: 8 },
      { type: 'turn_end', text: 'done' },
    ]);
    expect(reported.turn?.tools[8].status).toBe('ok');
    expect(reported.turn?.tools[8].summary).toBe('exit 0');
  });

  it('attaches a self-review to the card it names, by seq', () => {
    // Plan §4-A: the review runs a beat after the tool, so a card cannot be final at
    // tool_end. The join is `seq` — the same key the card is filed under — which is what
    // lets the badge appear on the card that earned it, whenever the review lands.
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'edit_file', args: { path: 'a.rs' }, seq: 3 },
      { type: 'tool_end', name: 'edit_file', ok: true, summary: 'ok', seq: 3 },
      { type: 'review', seq: 3, findings: [finding] },
    ]);
    expect(state.turn?.tools[3].findings).toEqual([finding]);
  });

  it("routes a nested agent's review to its step, not to the turn", () => {
    // A subagent's change is reviewed too, and its finding belongs on the nested step. The
    // turn's own card carries the same number here, which is the case that makes the author
    // load-bearing rather than decorative.
    const state = feed([
      { type: 'turn_start' },
      { type: 'subagent_start', id: 's1', label: 'research', depth: 1 },
      { type: 'tool_start', name: 'edit_file', args: {}, seq: 4, owner: 's1' },
      { type: 'tool_start', name: 'edit_file', args: {}, seq: 4 },
      { type: 'review', seq: 4, owner: 's1', findings: [finding] },
    ]);
    expect(state.subagents[0].steps[0].findings).toEqual([finding]);
    expect(state.turn?.tools[4].findings).toBeUndefined();
  });

  it('drops a review for a seq it has no card for', () => {
    // No phantom card: a review is a fact about a tool call, never a reason to invent one.
    const state = feed([{ type: 'turn_start' }, { type: 'review', seq: 9, findings: [] }]);
    expect(state.turn?.tools[9]).toBeUndefined();
  });


  it('builds a turn from a full event sequence', () => {
    const state = feed([
      { type: 'turn_start' },
      { type: 'text_delta', text: 'Hel' },
      { type: 'text_delta', text: 'lo' },
      { type: 'tool_start', name: 'read_file', args: { path: 'a.txt' }, seq: 0 },
      { type: 'tool_end', name: 'read_file', ok: true, summary: '1 file', seq: 0 },
      { type: 'turn_end', text: 'Hello' },
    ]);
    expect(state.running).toBe(false);
    // The finished turn is RETAINED (anti blank-flash) until turn_synced.
    expect(state.turn?.finished).toBe(true);
    expect(state.turn?.text).toBe('Hello');
    const synced = turnReducer(state, { type: 'turn_synced' });
    expect(synced.turn).toBeNull();
  });

  it('turn_end without a prior turn stays null', () => {
    const state = feed([{ type: 'turn_end', text: '' }]);
    expect(state.running).toBe(false);
    expect(state.turn).toBeNull();
  });

  it('tool_start records a startedAt timestamp', () => {
    const before = Date.now();
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'build', args: {}, seq: 0 },
    ]);
    expect(state.turn?.tools[0].startedAt).toBeGreaterThanOrEqual(before);
  });

  it('accumulates text deltas and marks tools running/ok', () => {
    const state = feed([
      { type: 'turn_start' },
      { type: 'text_delta', text: 'Hi' },
      { type: 'tool_start', name: 'grep', args: { pattern: 'fn' }, seq: 1 },
      { type: 'tool_end', name: 'grep', ok: true, summary: '3 matches', seq: 1 },
    ]);
    expect(state.running).toBe(true);
    expect(state.turn?.text).toBe('Hi');
    expect(state.turn?.tools[1]).toMatchObject({
      seq: 1,
      name: 'grep',
      status: 'ok',
      summary: '3 matches',
    });
  });

  it('marks a failed tool and keeps the rest of the turn intact', () => {
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_start', name: 'read_file', args: { path: 'x' }, seq: 0 },
      { type: 'tool_end', name: 'read_file', ok: false, summary: 'not found', seq: 0 },
      { type: 'text_delta', text: 'after tool' },
    ]);
    expect(state.turn?.tools[0].status).toBe('failed');
    expect(state.turn?.tools[0].summary).toBe('not found');
    expect(state.turn?.text).toBe('after tool');
  });

  it('ignores tool_end without a matching tool_start (stray event)', () => {
    const state = feed([
      { type: 'turn_start' },
      { type: 'tool_end', name: 'ghost', ok: true, summary: '', seq: 42 },
    ]);
    expect(state.turn?.tools[42]).toBeUndefined();
    expect(state.running).toBe(true);
  });

  it('surfaces an error when the turn never started', () => {
    const state = feed([{ type: 'error', message: 'no provider configured' }]);
    expect(state.running).toBe(false);
    expect(state.turn?.text).toContain('no provider configured');
  });

  it('appends an error to the running turn text', () => {
    const state = feed([
      { type: 'turn_start' },
      { type: 'text_delta', text: 'partial' },
      { type: 'error', message: 'network down' },
    ]);
    expect(state.turn?.text).toBe('partial\n⚠ network down');
  });

  it('does not mutate the previous state object', () => {
    const before = feed([{ type: 'turn_start' }]);
    const after = feed([{ type: 'text_delta', text: 'x' }], before);
    expect(before.turn?.text).toBe('');
    expect(after.turn?.text).toBe('x');
  });
});

describe('turnsReducer (multi-session routing)', () => {
  const start: FrontendEvent = { type: 'turn_start', session_id: 'a' };
  const deltaA: FrontendEvent = { type: 'text_delta', session_id: 'a', text: 'hi' };
  const startB: FrontendEvent = { type: 'turn_start', session_id: 'b' };
  const endA: FrontendEvent = { type: 'turn_end', session_id: 'a', text: '' };

  it('routes events to per-session slots independently', () => {
    let state = turnsReducer({}, start);
    state = turnsReducer(state, deltaA);
    // A different session starting must NOT touch session a's slot.
    state = turnsReducer(state, startB);
    expect(Object.keys(state).sort()).toEqual(['a', 'b']);
    expect(state.a.running).toBe(true);
    expect(state.b.running).toBe(true);
    expect(state.a.turn?.text).toBe('hi');
    expect(state.b.turn?.text).toBe('');
  });

  it('ends only the session that emitted turn_end (retained until synced)', () => {
    let state = turnsReducer({}, start);
    state = turnsReducer(state, startB);
    state = turnsReducer(state, endA);
    // The finished turn is RETAINED (anti blank-flash) — the slot stays
    // with running=false until turn_synced lands.
    expect(state.a?.running).toBe(false);
    expect(state.a?.turn?.finished).toBe(true);
    expect(state.b.running).toBe(true);
    const synced = turnsReducer(state, {
      type: 'turn_synced',
      session_id: 'a',
    });
    expect(synced.a).toBeUndefined();
    expect(synced.b.running).toBe(true);
  });

  it('ignores events without a session id (legacy/global)', () => {
    const orphan: FrontendEvent = { type: 'turn_start' };
    expect(turnsReducer({}, orphan)).toEqual({});
  });
});

describe('turnReducer thinking phase', () => {
  it('accumulates thinking and KEEPS it once text starts (collapsed preview)', () => {
    let s = turnReducer(initialTurnState(), { type: 'turn_start' });
    s = turnReducer(s, { type: 'thinking', text: 'pondering…', session_id: 'x' });
    expect(s.turn?.thinking).toBe('pondering…');
    s = turnReducer(s, { type: 'thinking', text: ' more', session_id: 'x' });
    expect(s.turn?.thinking).toBe('pondering… more');
    s = turnReducer(s, { type: 'text_delta', text: 'answer', session_id: 'x' });
    expect(s.turn?.text).toBe('answer');
    // Reasoning is retained for the collapsible preview — interleaved
    // reasoning between tool waves used to vanish with the rest.
    expect(s.turn?.thinking).toBe('pondering… more');
  });

  it('ignores thinking deltas when no turn is running', () => {
    const s = turnReducer(initialTurnState(), { type: 'thinking', text: '?', session_id: 'x' });
    expect(s.turn).toBeNull();
  });
});

describe('turn_synced (transcript replaces the live copy)', () => {
  it('leaves a still-running turn alone', () => {
    // App dispatches this whenever the open chat changes, and the chat being
    // opened may still be streaming: nulling its slot would make every later
    // delta a no-op and blank the reply the user just switched to read.
    let s = turnReducer(initialTurnState(), { type: 'turn_start' });
    s = turnReducer(s, { type: 'text_delta', text: 'half a repl' });
    const synced = turnReducer(s, { type: 'turn_synced' });
    expect(synced).toBe(s);
    expect(turnReducer(synced, { type: 'text_delta', text: 'y' }).turn?.text).toBe('half a reply');
  });

  it('drops a finished turn a background chat had retained', () => {
    // turn_end only refreshes the chat that is open, so a chat that finished
    // out of sight keeps its copy until it is reopened.
    let state = turnsReducer({}, { type: 'turn_start', session_id: 'bg' });
    state = turnsReducer(state, { type: 'text_delta', session_id: 'bg', text: 'answer' });
    state = turnsReducer(state, { type: 'turn_end', session_id: 'bg', text: 'answer' });
    expect(state.bg?.turn?.finished).toBe(true);
    state = turnsReducer(state, { type: 'turn_synced', session_id: 'bg' });
    expect(state.bg).toBeUndefined();
  });
});
