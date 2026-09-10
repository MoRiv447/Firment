import { describe, expect, it, vi } from 'vitest';
import { runAgentTurn, type AgentDeps, type ProviderLike } from '../agent';
import { accumulateToolCalls, type ToolCallChunk } from '../events';
import type { ChatMessage } from '../types';
import type { ChatRequest, StreamResult } from '../provider';

/**
 * Agent-loop tests. The provider and the tool executor are injected through
 * `deps`, so these run offline with no key and no network.
 */

const CONFIG = {
  defaultProvider: 'mock',
  providers: { mock: { type: 'openai', baseUrl: 'http://127.0.0.1:1/v1', apiKey: 'k', model: 'm' } },
  maxIterations: 5,
  contextBudgetChars: 60_000,
};

interface FakeProvider extends ProviderLike {
  /** Messages the model was asked with, one entry per call. */
  requests: ChatMessage[][];
}

function scriptedProvider(scripts: StreamResult[]): FakeProvider {
  const requests: ChatMessage[][] = [];
  let index = 0;
  return {
    requests,
    async streamChat(request: ChatRequest): Promise<StreamResult> {
      requests.push(request.messages);
      const script = scripts[Math.min(index, scripts.length - 1)];
      index++;
      return script;
    },
  };
}

function twoStep(calls: ReturnType<typeof accumulateToolCalls>): StreamResult[] {
  return [
    { text: '', toolCalls: calls },
    { text: 'final answer', toolCalls: [] },
  ];
}

function toolCalls(...names: string[]) {
  const chunks: ToolCallChunk[] = names.map((name, index) => ({
    index,
    id: `call_${index}`,
    name,
    args: JSON.stringify({ pattern: 'dma' }),
  }));
  return accumulateToolCalls(chunks);
}

function history(): ChatMessage[] {
  return [{ role: 'user', content: 'why does my DMA transfer stall?' }];
}

const toolResults = (messages: ChatMessage[]) => messages.filter((m) => m.role === 'tool');
const noopExecute = vi.fn() as unknown as AgentDeps['executeTool'];

describe('runAgentTurn cancellation (FIR-002)', () => {
  it('stops before the remaining tools in a batch and closes every tool call', async () => {
    const controller = new AbortController();
    const exec = vi.fn(
      async (
        name: string,
        _args: Record<string, any>,
        _cwd: string,
        _config: unknown,
        _signal?: AbortSignal
      ) => {
        controller.abort(); // client disconnects while the first tool runs
        return { success: true, output: `${name} done` };
      }
    );
    const executeTool = exec as unknown as AgentDeps['executeTool'];

    const provider = scriptedProvider(twoStep(toolCalls('grep', 'glob', 'read_file')));
    const result = await runAgentTurn(history(), 'q', CONFIG, undefined, controller.signal, {
      createProvider: () => provider,
      executeTool,
    });

    expect(exec).toHaveBeenCalledTimes(1);
    // The request signal must reach the executor, so an in-flight outbound
    // fetch aborts too — not just the calls queued behind it.
    expect(exec.mock.calls[0][4]).toBe(controller.signal);
    // The assistant message lists all three calls, so all three must be closed:
    // a dangling tool_calls entry would poison the session for every later
    // request.
    const assistant = result.newMessages.find((m) => m.role === 'assistant')!;
    const ids = (assistant.tool_calls || []).map((tc) => tc.id);
    expect(ids).toHaveLength(3);
    expect(toolResults(result.newMessages).map((m) => m.tool_call_id).sort()).toEqual([...ids].sort());
    expect(result.newMessages.filter((m) => m.content.includes('[Cancelled]'))).toHaveLength(2);
    // No second model call after the abort.
    expect(provider.requests).toHaveLength(1);
  });

  it('closes the whole batch when the client is already gone before any tool runs', async () => {
    const controller = new AbortController();
    controller.abort();
    const executeTool = vi.fn() as unknown as AgentDeps['executeTool'];
    const provider = scriptedProvider(twoStep(toolCalls('grep', 'glob')));

    const result = await runAgentTurn(history(), 'q', CONFIG, undefined, controller.signal, {
      createProvider: () => provider,
      executeTool,
    });

    expect(executeTool).not.toHaveBeenCalled();
    expect(toolResults(result.newMessages)).toHaveLength(2);
    expect(result.newMessages.some((m) => m.content.includes('[Cancelled]'))).toBe(true);
  });
});

describe('runAgentTurn with unusable arguments (FIR-003)', () => {
  it('refuses to execute a call whose streamed arguments were not valid JSON', async () => {
    const broken = accumulateToolCalls([
      { index: 0, id: 'call_0', name: 'grep', args: '{"pat' },
      { index: 0, args: 'tern": "dma"' }, // never closed
    ]);
    expect(broken[0].argsError).toBeDefined();
    expect(broken[0].arguments).toEqual({});

    const executeTool = vi.fn() as unknown as AgentDeps['executeTool'];
    const provider = scriptedProvider(twoStep(broken));
    const result = await runAgentTurn(history(), 'q', CONFIG, undefined, undefined, {
      createProvider: () => provider,
      executeTool,
    });

    expect(executeTool).not.toHaveBeenCalled();
    const closed = toolResults(result.newMessages)[0];
    expect(closed.content).toMatch(/\[InvalidArguments\]/);
    // …and the model still gets a matching tool result so it can retry.
    expect(closed.tool_call_id).toBe('call_0');
    // The persisted transcript keeps only the wire shape — no internal marker.
    const persisted = result.newMessages[0].tool_calls![0] as unknown as Record<string, unknown>;
    expect(Object.keys(persisted).sort()).toEqual(['arguments', 'id', 'name']);
  });
});

describe('runAgentTurn context budget notice', () => {
  it('emits exactly one info event when the context cannot be compacted further', async () => {
    const provider = scriptedProvider([{ text: 'ok', toolCalls: [] }]);
    const result = await runAgentTurn(
      [{ role: 'user', content: 'x'.repeat(12_000) }],
      'q',
      { ...CONFIG, contextBudgetChars: 1 },
      undefined,
      undefined,
      { createProvider: () => provider, executeTool: noopExecute }
    );

    const infos = result.events.filter((e) => e.type === 'info');
    expect(infos).toHaveLength(1);
    expect(infos[0].message).toMatch(/cannot be compacted further/);
    // The live request itself is never folded away.
    expect(provider.requests[0][1].content).toContain('xxx');
  });
});
