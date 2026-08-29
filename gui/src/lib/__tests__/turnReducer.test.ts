import { describe, expect, it } from 'vitest';
import { initialTurnState, turnReducer, turnsReducer, type TurnState } from '../turnReducer';
import type { FrontendEvent } from '../../types';

function feed(events: FrontendEvent[], start: TurnState = initialTurnState()): TurnState {
  return events.reduce(turnReducer, start);
}

describe('turnReducer (IDE event->UI contract)', () => {
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
  it('accumulates thinking then clears it when text starts', () => {
    let s = turnReducer(initialTurnState(), { type: 'turn_start' });
    s = turnReducer(s, { type: 'thinking', text: 'pondering…', session_id: 'x' });
    expect(s.turn?.thinking).toBe('pondering…');
    s = turnReducer(s, { type: 'thinking', text: ' more', session_id: 'x' });
    expect(s.turn?.thinking).toBe('pondering… more');
    s = turnReducer(s, { type: 'text_delta', text: 'answer', session_id: 'x' });
    // Reasoning done: the snippet must not linger under the reply.
    expect(s.turn?.text).toBe('answer');
    expect(s.turn?.thinking).toBe('');
  });

  it('ignores thinking deltas when no turn is running', () => {
    const s = turnReducer(initialTurnState(), { type: 'thinking', text: '?', session_id: 'x' });
    expect(s.turn).toBeNull();
  });
});
