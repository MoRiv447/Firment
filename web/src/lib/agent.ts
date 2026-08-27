import { FirmentProvider, ProviderConfig } from './provider';
import { executeTool } from './tools';
import { getSystemPrompt } from './system';
import { ChatMessage, ToolSpec, AgentEvent } from './types';
import { WEB_TOOL_SPECS } from './config';

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
  signal?: AbortSignal
): Promise<{ events: AgentEvent[]; finalText: string; newMessages: ChatMessage[] }> {
  const providerConfig = resolveProviderConfig(config);
  if (!providerConfig) {
    throw new Error('No provider configured — open Settings and configure the default provider first.');
  }
  const provider = buildProvider(providerConfig);

  const maxIterations = Math.min(Math.max(config?.maxIterations || DEFAULT_MAX_ITERATIONS, 1), 100);
  const contextBudget = Math.max(config?.contextBudgetChars || DEFAULT_CONTEXT_BUDGET, 10_000);

  const cwd = process.cwd();
  const systemPrompt = getSystemPrompt(cwd);
  const tools: ToolSpec[] = WEB_TOOL_SPECS;

  // history already includes the latest user message
  const messages: ChatMessage[] = [{ role: 'system', content: systemPrompt }, ...history];

  // Index where newly generated messages begin (after system + history).
  // Tracked explicitly so compaction of OLD history never invalidates it.
  let newStart = messages.length;

  const events: AgentEvent[] = [];
  let finalText = '';
  let iteration = 0;

  const emit = async (e: AgentEvent) => {
    events.push(e);
    if (onEvent) await onEvent(e);
  };

  await emit({ type: 'turn_start' });

  while (iteration < maxIterations) {
    iteration++;

    // Context compaction: keep the system prompt (index 0) and a recent tail
    // of history; only the OLD history (before this turn's new messages) is
    // compacted, so `newStart` stays valid. It can run again within the same
    // turn: after the first pass the layout is system, digest, tail(8); a
    // second pass folds the digest + tail into a single digest.
    const totalChars = messages.reduce((sum, m) => sum + (m.content?.length || 0), 0);
    if (totalChars > contextBudget && newStart > 2) {
      const digest = {
        role: 'user' as const,
        content:
          '[Context was compacted. Please continue helping with the current task based on the recent messages.]',
      };
      if (newStart > 10) {
        let tailStart = newStart - 8;
        // Never cut between an assistant(tool_calls) message and its `tool`
        // results: a retained history that STARTS with a bare `tool` message
        // is rejected by strict OpenAI-compatible providers ("role 'tool'
        // must respond to a preceding tool_calls"). Extend the tail back to
        // include the parent assistant instead.
        while (tailStart > 1 && messages[tailStart]?.role === 'tool') {
          tailStart--;
        }
        const tail = messages.slice(tailStart, newStart);
        messages.splice(1, tailStart - 1, digest);
        // After the splice the layout is: system, digest, tail, new messages…
        newStart = 2 + tail.length;
      } else {
        // Already compacted (or history too short): merge everything old
        // (digest + tail) into a single digest at index 1.
        messages.splice(1, newStart - 1, digest);
        newStart = 2;
      }
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

    // Preserve tool_calls so the model sees its own tool invocations in history
    messages.push({ role: 'assistant', content: text, tool_calls: toolCalls });

    // No tool calls -> we are done for this turn
    if (toolCalls.length === 0) {
      break;
    }

    // Client disconnected: stop before running more tools. The LLM stream is
    // aborted via the request signal; work already done stays in the transcript.
    if (signal?.aborted) {
      break;
    }

    // Execute each requested tool and feed results back as `tool` messages
    for (const tc of toolCalls) {
      await emit({ type: 'tool_start', toolName: tc.name });
      const toolResult = await executeTool(tc.name, tc.arguments, cwd, config);
      await emit({
        type: 'tool_end',
        toolName: tc.name,
        toolOutput: toolResult.output,
        toolOk: toolResult.success,
      });
      messages.push({
        role: 'tool',
        tool_call_id: tc.id,
        content: toolResult.success ? toolResult.output : `Error: ${toolResult.error || 'tool failed'}`,
      });
    }
    // Loop again so the model can act on tool results
  }

  // Return only the messages generated during this turn (assistant + tool pairs)
  const newMessages = messages.slice(newStart);

  await emit({ type: 'turn_end', text: finalText });

  return { events, finalText, newMessages };
}
