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
 *   produce broken entries. A missing index (some OpenAI-compatible servers
 *   omit it) falls back to the last used slot when the chunk continues an
 *   existing call, and to the next free slot when it carries a new id/name.
 * - `arguments` are JSON-fragments concatenated across deltas; a parse
 *   failure degrades to `{}` and is LOGGED — silently executing a tool with
 *   no arguments only produces a confusing downstream error.
 * - A `name` on a later chunk overwrites an empty first-chunk name (some
 *   servers fragment the function name across deltas).
 * - Entries without a name are dropped (an empty name means the delta only
 *   carried an id/args fragment with no function).
 */
export function accumulateToolCalls(chunks: ToolCallChunk[]): ParsedToolCall[] {
  const acc: Array<{ id: string; name: string; args: string }> = [];
  for (const tc of chunks) {
    let idx = tc.index;
    if (idx === undefined || idx === null || Number.isNaN(idx)) {
      const last = acc.length - 1;
      const continues =
        last >= 0 && !tc.id && !tc.name
          ? true // argument fragment: continue the most recent call
          : last >= 0 && !!tc.id && acc[last].id === tc.id;
      idx = continues ? last : acc.length;
    }
    if (!acc[idx]) {
      acc[idx] = { id: '', name: '', args: '' };
    }
    if (tc.id && !acc[idx].id) acc[idx].id = tc.id;
    if (tc.name && !acc[idx].name) acc[idx].name = tc.name;
    acc[idx].args += tc.args || '';
  }
  const parsed: ParsedToolCall[] = [];
  for (const tc of acc.filter(Boolean)) {
    let args: Record<string, any> = {};
    try {
      args = JSON.parse(tc.args || '{}');
    } catch {
      console.error(
        `tool call "${tc.name}" (${tc.id}): arguments are not valid JSON — executing with {}`,
        tc.args.slice(0, 200),
      );
      args = {};
    }
    if (tc.name) {
      parsed.push({ id: tc.id, name: tc.name, arguments: args });
    }
  }
  return parsed;
}
