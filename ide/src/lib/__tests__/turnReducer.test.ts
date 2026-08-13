import { describe, expect, it } from 'vitest';
import { initialTurnState, turnReducer, type TurnState } from '../turnReducer';
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
    expect(state.turn).toBeNull();
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
