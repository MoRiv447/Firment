import type { ChatMessage, ToolCall } from '../types';

/**
 * The transcript's shape, computed before anything is rendered.
 *
 * Both functions here are pure because the interesting decisions are about
 * boundaries -- where a run starts, where it stops, which answer belongs to which
 * call -- and a boundary is a lot easier to assert on than to eyeball in a
 * browser. They moved out of `components/MessageList.tsx` when the Changes pane
 * started needing the second one: a pane that lists the files a session edited
 * has to pair calls with results exactly the way the transcript does, and a
 * second, slightly different copy of that rule is how a file ends up listed
 * without the diff it was changed with.
 */

/** One entry in the transcript after grouping.
 *
 * The unit of the transcript used to be the *message*, which is why it read like
 * a log. A run of tool work is one event in the story, so it is one row that
 * opens. */
export type TranscriptRow =
  | { kind: 'single'; key: string; message: ChatMessage }
  | { kind: 'run'; key: string; messages: ChatMessage[] };

/** Assistant messages that only carry tool calls, and the results answering
 *  them. A message with prose is never part of a run: the prose is the point. */
function isRunMember(m: ChatMessage): boolean {
  if (m.role === 'tool') return true;
  return m.role === 'assistant' && !!m.tool_calls?.length && !m.content.trim();
}

/**
 * Collapse consecutive tool work into single rows, everything else untouched.
 *
 * Pure, so the shape can be asserted without rendering: the interesting cases
 * are the boundaries -- a run interrupted by a paragraph of prose is two runs,
 * not one, because the prose is what the reader is looking for.
 */
export function groupTranscript(messages: ChatMessage[]): TranscriptRow[] {
  const rows: TranscriptRow[] = [];
  let run: ChatMessage[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const first = run[0];
    const key = `run-${first.tool_calls?.[0]?.id ?? first.tool_call_id ?? rows.length}`;
    rows.push({ kind: 'run', key, messages: run });
    run = [];
  };
  messages.forEach((m, i) => {
    if (isRunMember(m)) {
      run.push(m);
      return;
    }
    flush();
    rows.push({ kind: 'single', key: `m${i}-${m.role}`, message: m });
  });
  flush();
  return rows;
}

/** A call and, when the transcript has it, the text that came back. */
export interface RunEntry {
  call: ToolCall;
  /** `undefined` while the result has not arrived (a live run) or when the
   *  stored transcript never paired them. */
  result?: string;
}

/**
 * Pair each tool call in a run with the tool message that answers it.
 *
 * The pairing key is the call id the backend assigns, which is the only thing
 * that survives a session being written to disk and read back. Results that no
 * call in this run claims come back separately rather than being dropped: the
 * transcript on disk can be interrupted by a paragraph of prose between a call
 * and its answer, and then the answer arrives in the run *after* the call.
 */
export function pairRun(messages: ChatMessage[]): {
  entries: RunEntry[];
  orphans: ChatMessage[];
} {
  const results = new Map<string, string>();
  for (const m of messages) {
    if (m.role === 'tool' && m.tool_call_id) results.set(m.tool_call_id, m.content);
  }
  const entries: RunEntry[] = [];
  for (const m of messages) {
    if (m.role !== 'assistant') continue;
    for (const call of m.tool_calls ?? []) {
      const result = call.id ? results.get(call.id) : undefined;
      if (result !== undefined) results.delete(call.id!);
      entries.push({ call, result });
    }
  }
  const orphans = messages.filter(
    (m) => m.role === 'tool' && (!m.tool_call_id || results.has(m.tool_call_id)),
  );
  return { entries, orphans };
}

/** Every call in the transcript, paired. A whole session is one long run here:
 *  the Changes pane wants what was touched, not where the paragraphs fell. */
export function pairAll(messages: ChatMessage[]): RunEntry[] {
  return pairRun(messages).entries;
}
