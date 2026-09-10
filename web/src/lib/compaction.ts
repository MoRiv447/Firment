import { ChatMessage } from './types';

/**
 * Context compaction for the web agent loop.
 *
 * Pure and side-effect free so the invariant it exists to protect can be
 * unit-tested: compaction folds OLD history away but must never remove the
 * live user request that the turn is answering.
 */

export interface CompactionResult {
  messages: ChatMessage[];
  newStart: number;
  compacted: boolean;
}

/** Size of one message as the provider sees it, including tool-call arguments. */
export function messageChars(m: ChatMessage): number {
  const callChars = m.tool_calls && m.tool_calls.length > 0 ? JSON.stringify(m.tool_calls).length : 0;
  return (m.content?.length || 0) + callChars;
}

export function totalChars(messages: ChatMessage[]): number {
  return messages.reduce((sum, m) => sum + messageChars(m), 0);
}

/**
 * Fold everything older than the last user message into a marker merged INTO
 * that message, keeping `[system, …recent]` well-formed.
 *
 * Two rules carry the fix:
 * - The cut point is the last `user` message before `newStart`, so the
 *   request being answered can never be compacted away. Cutting on a `user`
 *   boundary also means an `assistant(tool_calls)` message is never separated
 *   from its `tool` results (both are on the same side of the cut).
 * - The marker is merged into the surviving user message instead of being
 *   inserted as an extra `user` message: providers that enforce role
 *   alternation reject `[user, user]`.
 *
 * Once folded, the live request sits at index 1, so a second pass in the same
 * turn is a no-op — the budget is reported as un-compactable rather than
 * churning the transcript further.
 */
export function compactMessages(
  messages: ChatMessage[],
  newStart: number,
  budgetChars: number
): CompactionResult {
  const noop: CompactionResult = { messages, newStart, compacted: false };
  if (totalChars(messages) <= budgetChars) return noop;

  let lastUser = -1;
  for (let i = newStart - 1; i >= 1; i--) {
    if (messages[i].role === 'user') {
      lastUser = i;
      break;
    }
  }
  // Nothing "old" exists to fold (or no user message at all): dropping
  // anything here would take the live request with it.
  if (lastUser <= 1) return noop;

  const removedCount = lastUser - 1;
  const anchor = messages[lastUser];
  const next: ChatMessage[] = [
    messages[0],
    {
      ...anchor,
      content:
        `[Context was compacted: ${removedCount} earlier message(s) are no longer shown. ` +
        `Please continue helping with the current task below.]\n\n${anchor.content}`,
    },
    ...messages.slice(lastUser + 1),
  ];
  return { messages: next, newStart: newStart - removedCount, compacted: true };
}
