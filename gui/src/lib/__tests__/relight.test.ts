import { describe, expect, it } from 'vitest';

import { relightable } from '../relight';
import { initialTurnState, turnsReducer } from '../turnReducer';
import type { TurnMap } from '../turnReducer';
import type { TurnFlowEvent } from '../../types';

/**
 * The mount re-light, against the one ordering the window cannot control.
 *
 * `App` asks the backend which sessions have a live turn and lights each answer. The reply is an
 * IPC round trip behind the question, and a `turn_end` is a SEPARATE delivery from it — so the
 * event can arrive first, be dispatched into a reducer that has no slot for the turn yet, and then
 * be followed by a re-light that opens one. Nothing that turn does can ever close it again: its
 * end has already been and gone.
 *
 * The backend half of this (holding a turn's closing notice until the slot is released) is tested
 * in `gui/src-tauri/src/ui.rs`. What is tested here is what the window does with the remainder.
 */

const reduced = (events: TurnFlowEvent[]): TurnMap =>
  events.reduce<TurnMap>((state, event) => turnsReducer(state, event), {});

describe('relightable (a snapshot that lands after what it describes)', () => {
  it('lights every turn that is still running when the window opens', () => {
    expect(relightable(['a', 'b'], new Set<string>())).toEqual(['a', 'b']);
  });

  it('drops only the session whose end has already come through', () => {
    expect(relightable(['a', 'b'], new Set(['a']))).toEqual(['b']);
  });

  it('leaves a dropped turn unlit, which is the point of dropping it', () => {
    // The two deliveries in the order that breaks: the notice first, into a map with no slot for
    // the turn — the reducer has already seen its end and prunes it — and then the re-light.
    const afterEnd = reduced([{ type: 'turn_end', session_id: 'a', text: 'done' }]);
    const afterRelight = relightable(['a'], new Set(['a'])).reduce<TurnMap>(
      (state, id) => turnsReducer(state, { type: 'turn_start', session_id: id }),
      afterEnd,
    );
    expect(afterRelight.a ?? initialTurnState()).toMatchObject({ running: false });

    // The same sequence with the filter left out is the bug, spelled out rather than described:
    // a slot that says running, for a turn that finished before this window knew it existed.
    const unguarded = turnsReducer(afterEnd, { type: 'turn_start', session_id: 'a' });
    expect(unguarded.a?.running).toBe(true);
  });

  it('still lights a turn whose only traffic so far was not an end', () => {
    // A running turn streams deltas and tool calls, and none of those mean it is over. Dropping
    // the re-light for them would put this filter back where it started: no spinner, input live.
    const live = reduced([
      { type: 'text_delta', session_id: 'a', text: 'compiling' },
      { type: 'tool_start', session_id: 'a', name: 'build', args: {}, seq: 1 },
    ]);
    expect(live.a).toBeUndefined();
    const lit = relightable(['a'], new Set()).reduce<TurnMap>(
      (state, id) => turnsReducer(state, { type: 'turn_start', session_id: id }),
      live,
    );
    expect(lit.a?.running).toBe(true);
  });
});
