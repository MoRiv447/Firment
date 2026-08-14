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
      return { running: true, turn: { text: '', tools: {}, startedAt: Date.now() } };

    case 'text_delta':
      return state.turn
        ? { ...state, turn: { ...state.turn, text: state.turn.text + e.text } }
        : state;

    case 'tool_start':
      if (!state.turn) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: {
            ...state.turn.tools,
            [e.seq]: { seq: e.seq, name: e.name, args: e.args, status: 'running' },
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
      // Turn text is cleared so the transcript (refreshed by App.tsx) is
      // the single source of truth — the same reply must not show twice.
      return { running: false, turn: null };

    case 'error':
      // If the turn never started (e.g. no provider configured) `turn` is
      // null and the error must still surface instead of being dropped.
      return {
        running: false,
        turn: state.turn
          ? { ...state.turn, text: `${state.turn.text}\n⚠ ${e.message}` }
          : { text: `⚠ ${e.message}`, tools: {}, startedAt: Date.now() },
      };

    default:
      return state;
  }
}
