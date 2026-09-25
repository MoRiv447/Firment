import type { RunningTurn, ToolCardState, TurnFlowEvent } from '../types';

/**
 * Event -> UI state reducer for a running agent turn.
 *
 * This is the pure part of App.tsx's onAgentEvent switch: it maps the
 * agent event stream (turn_start / text_delta / tool_start / tool_end /
 * turn_end / error) to the running-turn UI state. Keeping it pure makes
 * the event->UI contract unit-testable without Tauri/React; App.tsx
 * drives it through useReducer and keeps the side effects (transcript
 * refresh on turn_end) outside.
 *
 * It also owns **subagent attribution**. A nested agent shares the parent's event sink, and each
 * agent numbers its calls from its own session, so `#3` can be two different cards at once. Every
 * card-addressed event therefore names its author (`owner`), and that is what routes a start, an
 * end, a phase and a finding badge to the right list. The stack of open agents remains, but only
 * for what carries no author: a nested run's prose and reasoning, which arrive as plain deltas
 * between its `subagent_start` and `subagent_end`.
 */

/** A nested agent, as the UI needs it: what it was asked, and what it did. */
export interface SubagentState {
  id: string;
  /** A short form of the prompt -- the question, which is the only useful name. */
  label: string;
  /** Nesting level; 1 is the first level of delegation. */
  depth: number;
  startedAt: number;
  steps: ToolCardState[];
  /** The nested run has returned. Kept in the list: it is the record of what
   *  ran, and a subagent that vanished on completion would leave the pane blank
   *  exactly when you go looking for its report. */
  done: boolean;
}

export interface TurnState {
  running: boolean;
  turn: RunningTurn | null;
  /** Every nested agent this turn started, oldest first. */
  subagents: SubagentState[];
  /** Ids of the subagents currently open, outermost first. A stack rather than
   *  a flag because subagents can spawn subagents: the innermost one owns the
   *  events that arrive while it is open. */
  stack: string[];
}

export function initialTurnState(): TurnState {
  return { running: false, turn: null, subagents: [], stack: [] };
}

/**
 * Apply `update` to the step list of the agent that issued the call, or `null` when the call
 * belongs to the turn itself (or to an author this UI has never heard of — a card is better
 * merged than dropped, so it lands on the turn and stays visible).
 *
 * This used to read the innermost open stack frame instead. That cannot work: subagents run in
 * parallel (`MAX_CONCURRENT_SUBAGENTS`), so with two frames open the innermost one would take
 * both agents' calls, and each agent numbers its own calls from its own session, so the numbers
 * collide as well.
 */
function routeToSubagent(
  state: TurnState,
  owner: string | null | undefined,
  update: (steps: ToolCardState[]) => ToolCardState[],
): SubagentState[] | null {
  if (!owner) return null;
  if (!state.subagents.some((s) => s.id === owner)) return null;
  return state.subagents.map((s) => (s.id === owner ? { ...s, steps: update(s.steps) } : s));
}

/** Resolve a card that is still running when its turn ends.
 *
 * Every path through the agent emits a `tool_end` — normal, cancelled and wave-timeout — so
 * reaching `turn_end` with a card still running means the event was lost, not that the tool is
 * still working. Saying so is better than a spinner that never stops, and worse than lying about
 * what it did, so the summary names the absence rather than a result.
 */
function closeRunning(t: ToolCardState): ToolCardState {
  return t.status === 'running'
    ? {
        ...t,
        status: 'failed',
        progress: undefined,
        endedAt: Date.now(),
        summary: 'no result reported before the turn ended',
      }
    : t;
}

/** Close every open frame. Used on the paths where the turn ends without the
 *  nested agents having reported back: a frame left open would attribute the
 *  NEXT turn's first tool calls to an agent that had already returned. */
function closeAll(state: TurnState): Pick<TurnState, 'subagents' | 'stack'> {
  return {
    subagents: state.subagents.map((s) =>
      s.done ? s : { ...s, done: true, steps: s.steps.map(closeRunning) },
    ),
    stack: [],
  };
}

export function turnReducer(state: TurnState, e: TurnFlowEvent): TurnState {
  switch (e.type) {
    case 'turn_start':
      // A new turn starts a new run: the previous turn's subagents are no longer
      // live, and keeping them would make the pane read as if they still were.
      return {
        running: true,
        turn: { text: '', thinking: '', tools: {}, startedAt: Date.now() },
        subagents: [],
        stack: [],
      };

    case 'subagent_start':
      return {
        ...state,
        subagents: [
          ...state.subagents,
          {
            id: e.id,
            label: e.label,
            depth: e.depth,
            startedAt: Date.now(),
            steps: [],
            done: false,
          },
        ],
        stack: [...state.stack, e.id],
      };

    case 'subagent_end': {
      // Pop by id, not by position: an end that arrives out of order must not
      // pop somebody else's frame and re-attribute the rest of the turn.
      const at = state.stack.indexOf(e.id);
      if (at < 0) {
        // An id we never saw open. Mark it done if it is known at all, but do
        // not touch the stack: popping a guess would misattribute the next call.
        return {
          ...state,
          subagents: state.subagents.map((s) => (s.id === e.id ? { ...s, done: true } : s)),
        };
      }
      // Everything from this frame inward closes with it -- a nested agent
      // cannot outlive the one that spawned it. Marking only `e.id` done left
      // the inner frames' state saying "still running" while their stack entry
      // was gone, so the pane would show a spinner forever.
      const closing = new Set(state.stack.slice(at));
      return {
        ...state,
        stack: state.stack.slice(0, at),
        subagents: state.subagents.map((s) => (closing.has(s.id) ? { ...s, done: true } : s)),
      };
    }

    case 'thinking': {
      if (!state.turn) return state;
      // A subagent's reasoning belongs to the subagent. The turn's `thinking`
      // is the main agent's, and mixing them is the whole problem this stack
      // exists to solve.
      if (state.stack.length > 0) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          // Cap the buffer: a long reasoning phase must not grow memory
          // unbounded — the UI only shows the tail anyway.
          thinking: (state.turn.thinking + e.text).slice(-2000),
        },
      };
    }

    case 'text_delta':
      if (!state.turn) return state;
      // Same rule: a subagent's prose is not the main agent's answer.
      if (state.stack.length > 0) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          // Reasoning is KEPT (rendered as a collapsed "reasoning" block
          // next to the reply) — wiping it on first text hid the
          // interleaved reasoning that arrives between tool waves for the
          // rest of the turn.
          text: state.turn.text + e.text,
        },
      };

    case 'tool_start': {
      if (!state.turn) return state;
      const card: ToolCardState = {
        seq: e.seq,
        name: e.name,
        args: e.args,
        status: 'running',
        startedAt: Date.now(),
      };
      const subagents = routeToSubagent(state, e.owner, (steps) => [...steps, card]);
      if (subagents) return { ...state, subagents };
      return { ...state, turn: { ...state.turn, tools: { ...state.turn.tools, [e.seq]: card } } };
    }

    case 'progress': {
      // Joined by `seq` + `owner`, like the review: the card is filed under that key in that
      // agent's list, so a phase that arrives before or after its tool_end still lands on the
      // right one.
      if (!state.turn) return state;
      const patch = (t: ToolCardState): ToolCardState => ({ ...t, progress: e.phase });
      const subagents = routeToSubagent(state, e.owner, (steps) =>
        steps.map((t) => (t.seq === e.seq ? patch(t) : t)),
      );
      if (subagents) return { ...state, subagents };
      if (!state.turn.tools[e.seq]) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: { ...state.turn.tools, [e.seq]: patch(state.turn.tools[e.seq]) },
        },
      };
    }

    case 'review': {
      // The self-review, joined to its card by `seq` + `owner` — the same key the card is filed
      // under, so a review that arrives after its tool_end still lands on the right card. A
      // nested agent's change is reviewed too, and its finding belongs on the nested step, not on
      // the turn's card that happens to carry the same number.
      if (!state.turn) return state;
      const patch = (t: ToolCardState): ToolCardState => ({ ...t, findings: e.findings });
      const subagents = routeToSubagent(state, e.owner, (steps) =>
        steps.map((t) => (t.seq === e.seq ? patch(t) : t)),
      );
      if (subagents) return { ...state, subagents };
      if (!state.turn.tools[e.seq]) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: { ...state.turn.tools, [e.seq]: patch(state.turn.tools[e.seq]) },
        },
      };
    }

    case 'tool_end': {
      if (!state.turn) return state;
      const patch = (t: ToolCardState): ToolCardState => ({
        ...t,
        status: e.ok ? 'ok' : 'failed',
        summary: e.summary,
        detail: e.detail,
        // The phase means "what it is doing now", so a finished tool has none: leaving it set
        // would keep an earlier sentence on screen next to the result mark.
        progress: undefined,
        // Same update as the status, so the duration a card reports and the outcome
        // it reports are one fact instead of two that could drift.
        endedAt: Date.now(),
        // The person's share of the card's wall time, taken at the same moment.
        waitedMs: e.waited_ms ?? null,
      });
      const subagents = routeToSubagent(state, e.owner, (steps) =>
        steps.map((t) => (t.seq === e.seq ? patch(t) : t)),
      );
      if (subagents) return { ...state, subagents };
      if (!state.turn.tools[e.seq]) return state;
      return {
        ...state,
        turn: {
          ...state.turn,
          tools: { ...state.turn.tools, [e.seq]: patch(state.turn.tools[e.seq]) },
        },
      };
    }

    case 'turn_end':
      // KEEP the finished turn rendered: clearing it here blanks the reply
      // until the post-turn transcript fetch lands (a visible blink, or a
      // lost reply when that fetch fails). App dispatches `turn_synced`
      // once the fresh transcript is committed, and THIS is where the turn
      // is finally dropped — the running flag flips now so the input
      // re-enables and the spinner row disappears.
      //
      // Every open subagent closes with it: a nested run cannot outlive the
      // parent that spawned it.
      return {
        running: false,
        turn: state.turn
          ? {
              ...state.turn,
              finished: true,
              tools: Object.fromEntries(
                Object.entries(state.turn.tools).map(([seq, t]) => [seq, closeRunning(t)]),
              ),
            }
          : null,
        ...closeAll(state),
      };

    case 'turn_synced':
      // The refreshed transcript now contains the reply; drop the retained
      // live copy (same React batch as setSession, so no double render).
      // A turn still RUNNING must survive this: switching chats reloads the
      // transcript of a chat that may be mid-stream, and every later delta
      // is ignored once the slot has no turn to append to.
      //
      // `subagents` SURVIVES the drop: it is the record of what ran, and the
      // inspector shows it until the next turn replaces it.
      if (state.running) return state;
      return { ...state, turn: null };

    case 'error':
      // If the turn never started (e.g. no provider configured) `turn` is
      // null and the error must still surface instead of being dropped.
      // Tools still marked running are resolved as failed — an interrupted
      // wave never gets its tool_end, and blue "running" cards would
      // otherwise hang under the transcript until the next turn.
      return {
        running: false,
        turn: state.turn
          ? {
              ...state.turn,
              text: `${state.turn.text}\n⚠ ${e.message}`,
              tools: Object.fromEntries(
                Object.entries(state.turn.tools).map(([seq, t]) => [
                  seq,
                  t.status === 'running'
                    ? { ...t, status: 'failed' as const, summary: 'interrupted by error' }
                    : t,
                ]),
              ),
            }
          : { text: `⚠ ${e.message}`, thinking: '', tools: {}, startedAt: Date.now() },
        ...closeAll(state),
      };

    default: {
      // Reaching this arm means a kind is in `TURN_FLOW_KINDS` with no case
      // above: `e` is not `never`, so the annotation below is the compile
      // error. At runtime the event came from a Rust process whose idea of the
      // union may be ahead of ours, so it is reported and dropped rather than
      // turned into state — a silent drop here is what an unwired event looks
      // like, and that is the bug this arm exists to make impossible.
      const unwired: never = e;
      console.error('turnReducer: unwired event kind', unwired);
      return state;
    }
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

export function turnsReducer(state: TurnMap, e: TurnFlowEvent): TurnMap {
  const sid = e.session_id || undefined;
  if (!sid) return state;
  const current = state[sid] ?? initialTurnState();
  const next = turnReducer(current, e);
  if (next === current) return state;
  // Prune finished-clean slots: a null turn with no error text and no subagent
  // record carries no information and would accumulate over a long-lived app.
  // Error turns are kept (their text is the only record until the next
  // transcript refresh), and so is a subagent list -- that is what the inspector
  // shows between turns.
  if (!next.running && next.turn === null && next.subagents.length === 0) {
    if (!(sid in state)) return state;
    const { [sid]: _removed, ...rest } = state;
    return rest;
  }
  return { ...state, [sid]: next };
}
