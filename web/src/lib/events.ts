/**
 * Event-contract helpers for the agent stream.
 *
 * These pure functions sit between the raw provider stream and the UI:
 * they turn chunked streaming data into the canonical AgentEvent payloads.
 * Kept dependency-free so they are trivially unit-testable and can be
 * shared verbatim with the IDE surface.
 */

export interface ToolCallChunk {
  index: number;
  id?: string;
  name?: string;
  args?: string;
}

export interface ParsedToolCall {
  id: string;
  name: string;
  arguments: Record<string, any>;
}

/**
 * Accumulate fragmented tool-call chunks (OpenAI stream deltas) into
 * complete tool calls.
 *
 * - `chunks` are indexed by `tc.index`; holes are tolerated (filter(Boolean)
 *   drops undefined entries) so a non-contiguous index sequence cannot
 *   produce broken entries.
 * - `arguments` are JSON-fragments concatenated across deltas; a parse
 *   failure degrades to `{}` rather than throwing (the model may emit
 *   partial JSON in edge cases; the caller logs and continues).
 * - Entries without a name are dropped (an empty name means the delta only
 *   carried an id/args fragment with no function).
 */
export function accumulateToolCalls(chunks: ToolCallChunk[]): ParsedToolCall[] {
  const acc: Array<{ id: string; name: string; args: string }> = [];
  for (const tc of chunks) {
    if (!acc[tc.index]) {
      acc[tc.index] = { id: tc.id || '', name: tc.name || '', args: '' };
    }
    acc[tc.index].args += tc.args || '';
  }
  const parsed: ParsedToolCall[] = [];
  for (const tc of acc.filter(Boolean)) {
    let args: Record<string, any> = {};
    try {
      args = JSON.parse(tc.args || '{}');
    } catch {
      args = {};
    }
    if (tc.name) {
      parsed.push({ id: tc.id, name: tc.name, arguments: args });
    }
  }
  return parsed;
}
