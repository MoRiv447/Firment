import { describe, expect, it } from 'vitest';
import { compactMessages, totalChars } from '../compaction';
import type { ChatMessage, ToolCall } from '../types';

/**
 * Contract tests for context compaction (FIR-001). The invariant under test:
 * compaction may fold away OLD history, but the user request the turn is
 * answering must always survive, and the transcript must stay provider-valid.
 */

const LIVE_REQUEST = '请帮我排查 SPI 驱动里的 DMA 竞争';

function sys(): ChatMessage {
  return { role: 'system', content: 'you are an embedded coding agent' };
}
function user(content: string): ChatMessage {
  return { role: 'user', content };
}
function assistant(content: string, toolCalls?: ToolCall[]): ChatMessage {
  return toolCalls ? { role: 'assistant', content, tool_calls: toolCalls } : { role: 'assistant', content };
}
function tool(id: string, content = 'ok'): ChatMessage {
  return { role: 'tool', content, tool_call_id: id };
}
function filler(label: string, chars = 2000): ChatMessage {
  return { role: 'assistant', content: `${label}:${'x'.repeat(chars)}` };
}

function longHistory(): ChatMessage[] {
  return [
    sys(),
    user('earlier question about the clock tree'),
    filler('a1'),
    user('and one about the CAN peripheral'),
    filler('a2'),
    user(LIVE_REQUEST),
  ];
}

/** Every `tool` result must have its parent assistant call still present. */
function assertNoDanglingToolCalls(messages: ChatMessage[]) {
  const callIds = new Set(
    messages.flatMap((m) => (m.tool_calls || []).map((tc) => tc.id))
  );
  for (const m of messages) {
    if (m.role === 'tool') expect(callIds.has(m.tool_call_id || '')).toBe(true);
  }
  if (messages.length > 1) expect(messages[1].role).not.toBe('tool');
}

describe('compactMessages', () => {
  it('is a no-op when the only history is the live request', () => {
    const messages = [sys(), user(LIVE_REQUEST)];
    const result = compactMessages(messages, messages.length, 10);
    expect(result.compacted).toBe(false);
    expect(result.messages).toBe(messages);
    expect(result.newStart).toBe(messages.length);
  });

  it('keeps the live request when folding older history', () => {
    const messages = longHistory();
    const result = compactMessages(messages, messages.length, 100);
    expect(result.compacted).toBe(true);
    expect(result.messages[0].role).toBe('system');
    expect(result.messages.some((m) => m.content.includes(LIVE_REQUEST))).toBe(true);
    assertNoDanglingToolCalls(result.messages);
  });

  it('merges the digest into the live request instead of inserting a second user message', () => {
    const messages = longHistory();
    const { messages: out } = compactMessages(messages, messages.length, 100);
    expect(out[1].role).toBe('user');
    expect(out[1].content).toContain(LIVE_REQUEST);
    expect(out[1].content).toContain('Context was compacted');
    // One digest message folded into the one surviving user turn: no new
    // `user` message is introduced, so role alternation stays provider-safe.
    expect(out.filter((m) => m.role === 'user')).toHaveLength(1);
  });

  it('is idempotent — a second pass folds nothing further', () => {
    const messages = longHistory();
    const first = compactMessages(messages, messages.length, 100);
    const second = compactMessages(first.messages, first.newStart, 100);
    expect(second.compacted).toBe(false);
    expect(second.messages).toBe(first.messages);
  });

  it('keeps an assistant/tool-call block on the same side of the cut', () => {
    const call: ToolCall = { id: 'call_1', name: 'grep', arguments: { pattern: 'dma' } };
    const messages: ChatMessage[] = [
      sys(),
      user('older question'),
      filler('old'),
      assistant('looking it up', [call]),
      tool('call_1'),
      user(LIVE_REQUEST),
      assistant('generated this turn'),
    ];
    const result = compactMessages(messages, messages.length - 1, 100);
    expect(result.compacted).toBe(true);
    assertNoDanglingToolCalls(result.messages);
    // The turn's own messages must survive the fold and stay where newStart says.
    expect(result.messages[result.newStart].content).toBe('generated this turn');
    expect(result.messages.some((m) => m.content.includes(LIVE_REQUEST))).toBe(true);
  });

  it('does not compact while the budget still holds', () => {
    const messages = longHistory();
    const result = compactMessages(messages, messages.length, totalChars(messages) + 1);
    expect(result.compacted).toBe(false);
  });

  it('counts tool-call arguments toward the size that triggers compaction', () => {
    const withCalls: ChatMessage = assistant('', [
      { id: 'c1', name: 'write_file', arguments: { content: 'y'.repeat(3000) } },
    ]);
    expect(totalChars([withCalls])).toBeGreaterThan(3000);
    expect(totalChars([assistant('')])).toBe(0);
  });
});
