import type { FrontendEvent, RunningTurn } from '../types';

/**
 * Event -> UI state reducer for a running agent turn.
 *
 * This is the pure part of App.tsx's onAgentEvent switch: it maps the
 * agent event stream (turn_start / text_delta / tool_start / tool_end /
 * turn_end / error) to the running-turn UI state. Keeping it pure makes
 * the event->UI contract unit-testable without Tauri/React; App.tsx
 * drives it through useReducer and keeps the side effects (transcript
 * refresh on turn_end) outside.
 */

export interface TurnState {
  running: boolean;
  turn: RunningTurn | null;
}

export function initialTurnState(): TurnState {
  return { running: false, turn: null };
}

export function turnReducer(state: TurnState, e: FrontendEvent): TurnState {
  switch (e.type) {
    case 'turn_start':
      return { running: true, turn: { text: '', thinking: '', tools: {}, startedAt: Date.now() } };

    case 'thinking': {
      if (!state.turn) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          // Cap the buffer: a long reasoning phase must not grow memory
          // unbounded — the UI only shows the tail anyway.
          thinking: (state.turn.thinking + (e as { text: string }).text).slice(-2000),
        },
      };
    }

    case 'text_delta':
      if (!state.turn) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          // First real text ends the visible reasoning phase.
          thinking: '',
          text: state.turn.text + e.text,
        },
      };

    case 'tool_start':
      if (!state.turn) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: {
            ...state.turn.tools,
            [e.seq]: {
              seq: e.seq,
              name: e.name,
              args: e.args,
              status: 'running',
              startedAt: Date.now(),
            },
          },
        },
      };

    case 'tool_end':
      if (!state.turn || !state.turn.tools[e.seq]) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: {
            ...state.turn.tools,
            [e.seq]: {
              ...state.turn.tools[e.seq],
              status: e.ok ? 'ok' : 'failed',
              summary: e.summary,
            },
          },
        },
      };

    case 'turn_end':
      // KEEP the finished turn rendered: clearing it here blanks the reply
      // until the post-turn transcript fetch lands (a visible blink, or a
      // lost reply when that fetch fails). App dispatches `turn_synced`
      // once the fresh transcript is committed, and THIS is where the turn
      // is finally dropped — the running flag flips now so the input
      // re-enables and the spinner row disappears.
      return {
        running: false,
        turn: state.turn ? { ...state.turn, finished: true } : null,
      };

    case 'turn_synced':
      // The refreshed transcript now contains the reply; drop the retained
      // live copy (same React batch as setSession, so no double render).
      return { running: false, turn: null };

    case 'error':
      // If the turn never started (e.g. no provider configured) `turn` is
      // null and the error must still surface instead of being dropped.
      return {
        running: false,
        turn: state.turn
          ? { ...state.turn, text: `${state.turn.text}\n⚠ ${e.message}` }
          : { text: `⚠ ${e.message}`, thinking: '', tools: {}, startedAt: Date.now() },
      };

    default:
      return state;
  }
}

/**
 * Multi-session wrapper: parallel chats each own a TurnState keyed by
 * session id. The backend stamps turn-flow events with `session_id`; the
 * wrapper routes them to the right slot and delegates to the pure
 * single-session reducer above. Sessions without activity simply have no
 * entry.
 */
export type TurnMap = Record<string, TurnState>;

export function turnsReducer(state: TurnMap, e: FrontendEvent): TurnMap {
  const sid = (e as { session_id?: string | null }).session_id || undefined;
  if (!sid) return state;
  const current = state[sid] ?? initialTurnState();
  const next = turnReducer(current, e);
  if (next === current) return state;
  // Prune finished-clean slots: a null turn with no error text carries no
  // information and would accumulate over a long-lived app. Error turns are
  // kept (their text is the only record until the next transcript refresh).
  if (!next.running && next.turn === null) {
    if (!(sid in state)) return state;
    const { [sid]: _removed, ...rest } = state;
    return rest;
  }
  return { ...state, [sid]: next };
}
