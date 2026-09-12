import { describe, expect, it } from 'vitest';
import { initialTurnState, turnReducer } from '../turnReducer';
import type { TurnState } from '../turnReducer';
import type { FrontendEvent } from '../../types';

/**
 * Subagent attribution.
 *
 * A nested agent shares the parent's event sink, so its tool calls arrive on the
 * parent's stream stamped with the parent's session id. Before the
 * `subagent_start` / `subagent_end` pair existed they were indistinguishable
 * from the main agent's, and the transcript showed a research subagent's
 * `read_file` as if the main agent had made it.
 *
 * The stack is the mechanism, so these are the cases that decide whether it
 * holds: nesting, out-of-order ends, and a run that never reports back.
 */

function apply(events: FrontendEvent[], from: TurnState = initialTurnState()): TurnState {
  return events.reduce(turnReducer, from);
}

const start = (id: string, depth = 1): FrontendEvent => ({
  type: 'subagent_start',
  id,
  label: '研究 STM32F4 EXTI',
  depth,
});
const end = (id: string, depth = 1): FrontendEvent => ({
  type: 'subagent_end',
  id,
  depth,
});
const toolStart = (seq: number, name: string): FrontendEvent => ({
  type: 'tool_start',
  name,
  args: {},
  seq,
});
const toolEnd = (seq: number, name: string, ok = true): FrontendEvent => ({
  type: 'tool_end',
  name,
  ok,
  summary: `${name} done`,
  seq,
});
const turnStart: FrontendEvent = { type: 'turn_start' };

describe('subagent attribution', () => {
  it('routes a nested tool call to the subagent, not the turn', () => {
    const s = apply([
      turnStart,
      start('a'),
      toolStart(1, 'read_file'),
      toolEnd(1, 'read_file'),
      end('a'),
    ]);
    expect(s.turn?.tools).toEqual({});
    expect(s.subagents).toHaveLength(1);
    expect(s.subagents[0].steps.map((t) => t.name)).toEqual(['read_file']);
    expect(s.subagents[0].steps[0].status).toBe('ok');
  });

  it('routes the main agent’s own calls to the turn', () => {
    const s = apply([turnStart, toolStart(1, 'edit_file'), toolEnd(1, 'edit_file')]);
    expect(Object.keys(s.turn?.tools ?? {})).toEqual(['1']);
    expect(s.subagents).toEqual([]);
  });

  it('attributes to the innermost agent when subagents nest', () => {
    const s = apply([
      turnStart,
      start('outer'),
      toolStart(1, 'read_file'),
      start('inner', 2),
      toolStart(2, 'grep'),
      end('inner', 2),
      toolStart(3, 'list_dir'),
    ]);
    const outer = s.subagents.find((x) => x.id === 'outer');
    const inner = s.subagents.find((x) => x.id === 'inner');
    // `grep` ran inside the inner agent; `list_dir` resumed in the outer one,
    // which is what makes the stack worth having over a boolean flag.
    expect(outer?.steps.map((t) => t.name)).toEqual(['read_file', 'list_dir']);
    expect(inner?.steps.map((t) => t.name)).toEqual(['grep']);
  });

  it('keeps a finished subagent in the list', () => {
    // It is the record of what ran. Removing it on completion would empty the
    // pane exactly when you go looking for the report.
    const s = apply([turnStart, start('a'), end('a')]);
    expect(s.subagents).toHaveLength(1);
    expect(s.subagents[0].done).toBe(true);
  });

  it('closing a frame closes everything it spawned', () => {
    // A nested agent cannot outlive the agent that spawned it, so an `end` for
    // the outer frame takes the inner one with it rather than leaving a frame
    // open whose parent is gone -- which would attribute later calls to an agent
    // that no longer exists.
    const s = apply([
      turnStart,
      start('outer'),
      start('inner', 2),
      end('outer'),
      toolStart(1, 'grep'),
    ]);
    expect(s.stack).toEqual([]);
    expect(s.subagents.every((x) => x.done)).toBe(true);
    // ...and the call after it is the main agent's, which is the point: a
    // stranded frame would have swallowed it.
    expect(Object.keys(s.turn?.tools ?? {})).toEqual(['1']);
  });

  it('ignores an end for an id it never saw', () => {
    const s = apply([turnStart, start('a'), end('ghost'), toolStart(1, 'read_file')]);
    const a = s.subagents.find((x) => x.id === 'a');
    expect(a?.steps).toHaveLength(1);
  });

  it('closes every open frame when the turn ends', () => {
    // A nested run cannot outlive the parent. A frame left open would attribute
    // the NEXT turn's first tool calls to an agent that had already returned.
    const s = apply([turnStart, start('a'), { type: 'turn_end', text: '' }, toolStart(1, 'read_file')]);
    expect(s.stack).toEqual([]);
    expect(s.subagents[0].done).toBe(true);
  });

  it('closes every open frame on an error', () => {
    const s = apply([turnStart, start('a'), { type: 'error', message: 'provider died' }]);
    expect(s.stack).toEqual([]);
    expect(s.subagents[0].done).toBe(true);
  });

  it('keeps a subagent’s prose out of the main reply', () => {
    // The nested agent streams text too. Appending it to the turn's answer
    // would put the subagent's research notes inside the main agent's reply.
    const s = apply([
      turnStart,
      start('a'),
      { type: 'text_delta', text: 'subagent note' },
      { type: 'thinking', text: 'subagent reasoning' },
      end('a'),
      { type: 'text_delta', text: 'the answer' },
    ]);
    expect(s.turn?.text).toBe('the answer');
    expect(s.turn?.thinking).toBe('');
  });

  it('drops the previous turn’s subagents when a new turn starts', () => {
    const s = apply([turnStart, start('a'), end('a'), turnStart]);
    expect(s.subagents).toEqual([]);
    expect(s.stack).toEqual([]);
  });

  it('keeps the subagent record after the live turn is synced away', () => {
    const s = apply([
      turnStart,
      start('a'),
      toolStart(1, 'read_file'),
      end('a'),
      { type: 'turn_end', text: 'done' },
      { type: 'turn_synced' },
    ]);
    expect(s.turn).toBeNull();
    expect(s.subagents).toHaveLength(1);
    expect(s.subagents[0].steps).toHaveLength(1);
  });
});
