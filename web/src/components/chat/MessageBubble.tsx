'use client';

import { User, Wrench } from 'lucide-react';
import { ChatMessage } from '@/lib/types';

/**
 * One message, from either side of the conversation.
 *
 * Extracted from `ChatPage.tsx` verbatim. Two things here are load-bearing
 * rather than cosmetic:
 *
 *   * a user message is right-aligned and brand-filled, an assistant one is
 *     left-aligned and neutral. Which side a message is on is how you read a
 *     transcript without reading it.
 *   * the tool chips name the tool, and are rendered for BOTH roles, because the
 *     transcript keeps the tool calls a turn made.
 */
export function MessageBubble({
  message,
  streaming,
}: {
  message: ChatMessage;
  streaming?: boolean;
}) {
  const isUser = message.role === 'user';
  const toolNames = message.tool_calls?.map((tc) => tc.name).filter(Boolean) || [];

  return (
    <div className={`flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
      {isUser ? (
        <div className="w-8 h-8 flex items-center justify-center shrink-0 bg-gray-700 border-[2px] border-black">
          <User className="w-4 h-4 text-gray-300" />
        </div>
      ) : (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src="/logo-w-32.png"
          alt="Firment"
          className="w-8 h-8 shrink-0 object-contain surface-brand p-0.5 border-[2px] border-black"
        />
      )}
      <div
        style={{ width: 'fit-content', maxWidth: '85%' }}
        className={`px-3 py-2 border-[3px] border-black ${
          isUser ? 'surface-brand shadow-[4px_4px_0_#000]' : 'bg-gray-800 text-gray-100 border-[2px]'
        }`}
      >
        <div className="text-sm leading-relaxed whitespace-pre-wrap">
          {(message.content ?? '').replace(/^[\s\n]+|[\s\n]+$/g, '')}
        </div>
        {toolNames.length > 0 && (
          <div className="flex flex-wrap gap-1 mt-2">
            {toolNames.map((name, idx) => (
              <span
                key={idx}
                className="inline-flex items-center gap-1 px-2 py-0.5 bg-gray-700/70 text-gray-300 text-xs font-mono border-[1px] border-black"
              >
                <Wrench className="w-3 h-3" />
                {name}
              </span>
            ))}
          </div>
        )}
        {!isUser && streaming && <p className="text-xs text-gray-500 mt-2">generating…</p>}
      </div>
    </div>
  );
}
