import { FirmentProvider, ProviderConfig, ChatRequest, StreamResult } from './provider';
import { executeTool } from './tools';
import { getSystemPrompt } from './system';
import { ChatMessage, ToolSpec, AgentEvent } from './types';
import { WEB_TOOL_SPECS } from './config';
import { compactMessages, totalChars } from './compaction';

const DEFAULT_MAX_ITERATIONS = 30;
const DEFAULT_CONTEXT_BUDGET = 60_000;

function resolveProviderConfig(config: any): ProviderConfig | null {
  const providerName = config?.defaultProvider;
  const provider = config?.providers?.[providerName];
  if (!provider) return null;
  return {
    type: provider.type,
    baseUrl: provider.baseUrl,
    apiKey: provider.apiKey || '',
    model: provider.model,
    maxTokens: provider.maxTokens,
    temperature: provider.temperature,
  };
}

function buildProvider(config: ProviderConfig): FirmentProvider {
  return new FirmentProvider({
    type: config.type,
    baseUrl: config.baseUrl,
    apiKey: config.apiKey,
    model: config.model,
    maxTokens: config.maxTokens,
    temperature: config.temperature,
  });
}

/** The slice of the provider the loop actually uses — lets tests inject a fake. */
export interface ProviderLike {
  streamChat(
    request: ChatRequest,
    onEvent: (event: AgentEvent) => Promise<void>
  ): Promise<StreamResult>;
}

export interface AgentDeps {
  createProvider: (config: ProviderConfig) => ProviderLike;
  executeTool: typeof executeTool;
}

const DEFAULT_AGENT_DEPS: AgentDeps = { createProvider: buildProvider, executeTool };

/**
 * Run one agent turn. `history` already contains the full prior conversation
 * (including the latest user message). The server is stateless: it does not
 * keep session state between requests. It returns only the messages produced
 * during this turn so the client can append them to its own persisted history.
 */
export async function runAgentTurn(
  history: ChatMessage[],
  userInput: string,
  config: any,
  onEvent?: (event: AgentEvent) => void,
  signal?: AbortSignal,
  deps: AgentDeps = DEFAULT_AGENT_DEPS
): Promise<{ events: AgentEvent[]; finalText: string; newMessages: ChatMessage[] }> {
  const providerConfig = resolveProviderConfig(config);
  if (!providerConfig) {
    throw new Error('No provider configured — open Settings and configure the default provider first.');
  }
  const provider = deps.createProvider(providerConfig);

  const maxIterations = Math.min(Math.max(config?.maxIterations || DEFAULT_MAX_ITERATIONS, 1), 100);
  const contextBudget = Math.max(config?.contextBudgetChars || DEFAULT_CONTEXT_BUDGET, 10_000);

  const cwd = process.cwd();
  const systemPrompt = getSystemPrompt(cwd);
  const tools: ToolSpec[] = WEB_TOOL_SPECS;

  // history already includes the latest user message
  let messages: ChatMessage[] = [{ role: 'system', content: systemPrompt }, ...history];

  // Index where newly generated messages begin (after system + history).
  // Tracked explicitly so compaction of OLD history never invalidates it.
  let newStart = messages.length;

  const events: AgentEvent[] = [];
  let finalText = '';
  let iteration = 0;
  let overBudgetNoticed = false;

  const emit = async (e: AgentEvent) => {
    events.push(e);
    if (onEvent) await onEvent(e);
  };

  await emit({ type: 'turn_start' });

  while (iteration < maxIterations) {
    iteration++;

    // Context compaction (see compaction.ts): folds history older than the
    // last user message. It can run on every iteration; once folded, the live
    // request sits at index 1 and the helper becomes a no-op.
    const compacted = compactMessages(messages, newStart, contextBudget);
    if (compacted.compacted) {
      messages = compacted.messages;
      newStart = compacted.newStart;
    } else if (!overBudgetNoticed && totalChars(messages) > contextBudget) {
      overBudgetNoticed = true;
      await emit({
        type: 'info',
        message:
          `Context is ${totalChars(messages)} characters, over the ${contextBudget} budget, and cannot be ` +
          'compacted further (only the current request is left). The provider may reject this request.',
      });
    }

    const result = await provider.streamChat(
      {
        messages,
        tools,
        maxTokens: providerConfig.maxTokens,
        temperature: providerConfig.temperature,
        thinking: config?.thinking || 'off',
        signal,
      },
      async (event) => {
        await emit(event);
      }
    );

    const { text, toolCalls } = result;

    if (text) {
      finalText += text;
    }

    // Preserve tool_calls so the model sees its own tool invocations in
    // history. Copy the wire shape only: `argsError` is a this-turn execution
    // marker, and persisting it would leak an internal diagnostic into the
    // client transcript (and back into every later request body).
    messages.push({
      role: 'assistant',
      content: text,
      tool_calls: toolCalls.map((tc) => ({ id: tc.id, name: tc.name, arguments: tc.arguments })),
    });

    // No tool calls -> we are done for this turn
    if (toolCalls.length === 0) {
      break;
    }

    // The assistant message above carries the WHOLE batch, so every id in it
    // must get a matching `tool` result before we stop — a dangling
    // tool_calls entry poisons the transcript for every later request.
    const closeToolCall = async (id: string, name: string, note: string) => {
      await emit({ type: 'tool_end', toolName: name, toolOutput: note, toolOk: false });
      messages.push({ role: 'tool', tool_call_id: id, content: `Error: ${note}` });
    };

    // Cancellation is checked per tool, not per batch: a disconnected client
    // must not keep issuing outbound requests for the calls queued behind the
    // one that was in flight.
    let aborted = false;
    for (let i = 0; i < toolCalls.length; i++) {
      const tc = toolCalls[i];
      if (signal?.aborted) {
        aborted = true;
        for (const pending of toolCalls.slice(i)) {
          await closeToolCall(pending.id, pending.name, '[Cancelled] client disconnected, tool not executed');
        }
        break;
      }
      if (tc.argsError) {
        await closeToolCall(
          tc.id,
          tc.name,
          `[InvalidArguments] ${tc.argsError}; the tool was not executed. Re-send with complete arguments.`
        );
        continue;
      }
      await emit({ type: 'tool_start', toolName: tc.name });
      const toolResult = await deps.executeTool(tc.name, tc.arguments, cwd, config, signal);
      await emit({
        type: 'tool_end',
        toolName: tc.name,
        // The UI card reads `toolOutput`; without the error text a rejected
        // call (e.g. [InvalidInput]) renders as an unexplained blank.
        toolOutput: toolResult.success ? toolResult.output : toolResult.error || 'tool failed',
        toolOk: toolResult.success,
      });
      messages.push({
        role: 'tool',
        tool_call_id: tc.id,
        content: toolResult.success ? toolResult.output : `Error: ${toolResult.error || 'tool failed'}`,
      });
    }
    if (aborted) {
      break;
    }
    // Loop again so the model can act on tool results
  }

  // Return only the messages generated during this turn (assistant + tool pairs)
  const newMessages = messages.slice(newStart);

  await emit({ type: 'turn_end', text: finalText });

  return { events, finalText, newMessages };
}
