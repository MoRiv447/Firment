export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, any>;
}

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

export interface AgentEvent {
  type: 'text_delta' | 'tool_start' | 'tool_end' | 'tool_calls' | 'turn_start' | 'turn_end' | 'info' | 'error';
  text?: string;
  toolCall?: ToolCall;
  toolCalls?: ToolCall[];
  toolName?: string;
  toolOutput?: string;
  toolOk?: boolean;
  message?: string;
}

export interface ToolSpec {
  name: string;
  description: string;
  input_schema: any;
}

export interface Session {
  id: string;
  cwd: string;
  provider: string;
  model: string;
  messages: ChatMessage[];
  createdAt: number;
  updatedAt: number;
}

export interface MessageWithEvents extends ChatMessage {
  events?: AgentEvent[];
  isLoading?: boolean;
}
