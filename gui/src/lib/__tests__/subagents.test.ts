import { describe, expect, it } from 'vitest';
import { initialTurnState, turnReducer } from '../turnReducer';
import type { TurnState } from '../turnReducer';
import type { FrontendEvent } from '../../types';

/**
 * Subagent attribution.
 *
 * A nested agent shares the parent's event sink, and each agent numbers its own calls from its
 * own session — so `#1` can be two different cards in one turn. Every card-addressed event names
 * its author (`owner`), and that is what puts a start, an end, a phase and a finding badge on the
 * list belonging to the agent that made the call. Prose and reasoning carry no author, so the
 * open-agent stack still decides where those go.
 *
 * The cases that follow are the ones that decide whether it holds: nested frames, two agents
 * running side by side, an author this UI never heard of, and a run that never reports back.
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
const toolStart = (seq: number, name: string, owner?: string): FrontendEvent => ({
  type: 'tool_start',
  name,
  args: {},
  seq,
  owner: owner ?? null,
});
const toolEnd = (seq: number, name: string, ok = true, owner?: string): FrontendEvent => ({
  type: 'tool_end',
  name,
  ok,
  summary: `${name} done`,
  seq,
  owner: owner ?? null,
});
const turnStart: FrontendEvent = { type: 'turn_start' };

describe('subagent attribution', () => {
  it('routes a nested tool call to the subagent, not the turn', () => {
    const s = apply([
      turnStart,
      start('a'),
      toolStart(1, 'read_file', 'a'),
      toolEnd(1, 'read_file', true, 'a'),
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

  it('attributes each call to the agent that issued it, through nesting', () => {
    const s = apply([
      turnStart,
      start('outer'),
      toolStart(1, 'read_file', 'outer'),
      start('inner', 2),
      toolStart(2, 'grep', 'inner'),
      end('inner', 2),
      toolStart(3, 'list_dir', 'outer'),
    ]);
    const outer = s.subagents.find((x) => x.id === 'outer');
    const inner = s.subagents.find((x) => x.id === 'inner');
    // `grep` ran inside the inner agent; the other two belong to the outer one.
    // The numbers do not decide this, because each agent counts from one.
    expect(outer?.steps.map((t) => t.name)).toEqual(['read_file', 'list_dir']);
    expect(inner?.steps.map((t) => t.name)).toEqual(['grep']);
  });

  it('keeps two parallel agents with the same call number apart', () => {
    // The case a stack cannot answer: subagents run concurrently, so with two
    // frames open the innermost one would take both agents' `#1` — one card
    // would be closed by the other agent's end, and the first would spin.
    const s = apply([
      turnStart,
      start('a'),
      start('b'),
      toolStart(1, 'read_file', 'a'),
      toolStart(1, 'grep', 'b'),
      toolEnd(1, 'read_file', true, 'a'),
    ]);
    const a = s.subagents.find((x) => x.id === 'a');
    const b = s.subagents.find((x) => x.id === 'b');
    expect(a?.steps.map((t) => [t.name, t.status])).toEqual([['read_file', 'ok']]);
    expect(b?.steps.map((t) => [t.name, t.status])).toEqual([['grep', 'running']]);
  });

  it('shows a call whose author it never saw on the turn, rather than losing it', () => {
    const s = apply([turnStart, toolStart(1, 'read_file', 'ghost'), toolEnd(1, 'read_file', true, 'ghost')]);
    expect(Object.keys(s.turn?.tools ?? {})).toEqual(['1']);
    expect(s.turn?.tools['1'].status).toBe('ok');
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
    const s = apply([turnStart, start('a'), end('ghost'), toolStart(1, 'read_file', 'a')]);
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
      toolStart(1, 'read_file', 'a'),
      end('a'),
      { type: 'turn_end', text: 'done' },
      { type: 'turn_synced' },
    ]);
    expect(s.turn).toBeNull();
    expect(s.subagents).toHaveLength(1);
    expect(s.subagents[0].steps).toHaveLength(1);
  });
});
