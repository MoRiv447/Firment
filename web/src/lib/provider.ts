import OpenAI from 'openai';
import { ChatMessage, ToolSpec, AgentEvent, ToolCall } from './types';

export interface ProviderConfig {
  type: 'openai' | 'anthropic';
  baseUrl: string;
  apiKey: string;
  model: string;
  maxTokens?: number;
  temperature?: number;
}

export interface ChatRequest {
  model?: string;
  messages: ChatMessage[];
  tools: ToolSpec[];
  maxTokens?: number;
  temperature?: number;
  thinking?: string;
}

export interface StreamResult {
  text: string;
  toolCalls: ToolCall[];
}

export class FirmentProvider {
  private client: OpenAI;
  private config: ProviderConfig;

  constructor(config: ProviderConfig) {
    this.config = config;
    const opts: ConstructorParameters<typeof OpenAI>[0] = {
      baseURL: config.baseUrl || undefined,
      apiKey: config.apiKey,
    };
    // Anthropic 官方端点不是 OpenAI 格式，这里走 OpenAI 兼容网关。
    // 如果是 anthropic 类型且填的是官方地址，给出明确提示由调用方处理。
    this.client = new OpenAI(opts);
  }

  async streamChat(
    request: ChatRequest,
    onEvent: (event: AgentEvent) => Promise<void>
  ): Promise<StreamResult> {
    const tools = request.tools.map(spec => ({
      type: 'function' as const,
      function: {
        name: spec.name,
        description: spec.description,
        parameters: spec.input_schema,
      },
    }));

    const response = await this.client.chat.completions.create({
      model: request.model || this.config.model,
      messages: this.convertMessages(request.messages),
      tools: tools.length > 0 ? tools : undefined,
      stream: true,
      max_tokens: request.maxTokens || this.config.maxTokens,
      temperature: request.temperature ?? this.config.temperature,
    });

    let fullText = '';
    let toolCalls: Array<{ id: string; name: string; args: string }> = [];

    for await (const chunk of response) {
      const delta = chunk.choices[0]?.delta;

      if (delta?.content) {
        fullText += delta.content;
        await onEvent({ type: 'text_delta', text: delta.content });
      }

      if (delta?.tool_calls) {
        for (const tc of delta.tool_calls) {
          if (!toolCalls[tc.index]) {
            toolCalls[tc.index] = { id: tc.id!, name: tc.function?.name || '', args: '' };
          }
          toolCalls[tc.index].args += tc.function?.arguments || '';
        }
      }
    }

    const parsed: ToolCall[] = [];
    // toolCalls is an array indexed by `tc.index`; filter out any holes so a
    // non-contiguous index sequence cannot produce `undefined` entries.
    for (const tc of toolCalls.filter(Boolean)) {
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
    if (parsed.length > 0) {
      await onEvent({ type: 'tool_calls', toolCalls: parsed });
    }

    return { text: fullText, toolCalls: parsed };
  }

  private convertMessages(messages: ChatMessage[]): any[] {
    return messages.map(msg => {
      switch (msg.role) {
        case 'system':
          return { role: 'system', content: msg.content };
        case 'user':
          return { role: 'user', content: msg.content };
        case 'assistant':
          if (msg.tool_calls && msg.tool_calls.length > 0) {
            return {
              role: 'assistant',
              // OpenAI rejects empty-string content alongside tool_calls; use null.
              content: msg.content || null,
              tool_calls: msg.tool_calls.map(tc => ({
                id: tc.id,
                type: 'function',
                function: {
                  name: tc.name,
                  arguments: JSON.stringify(tc.arguments),
                },
              })),
            };
          }
          return { role: 'assistant', content: msg.content };
        case 'tool':
          return {
            role: 'tool',
            tool_call_id: msg.tool_call_id,
            content: msg.content,
          };
        default:
          return msg;
      }
    });
  }
}
