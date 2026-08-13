import { NextRequest } from 'next/server';
import { runAgentTurn } from '@/lib/agent';

export const dynamic = 'force-dynamic';

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { messages, userInput, config } = body;

    if (!Array.isArray(messages)) {
      return new Response(JSON.stringify({ error: 'messages array is required' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (!userInput?.trim()) {
      return new Response(JSON.stringify({ error: 'User input is required' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    const encoder = new TextEncoder();

    const stream = new ReadableStream({
      async start(controller) {
        const send = (obj: any) => {
          controller.enqueue(encoder.encode(`data: ${JSON.stringify(obj)}\n\n`));
        };

        try {
          const result = await runAgentTurn(
            messages,
            userInput,
            config || {},
            (event) => send(event)
          );
          send({ type: 'done', finalText: result.finalText, newMessages: result.newMessages });
        } catch (error: any) {
          console.error('Chat error:', error);
          send({ type: 'error', error: error?.message || 'Internal server error' });
        } finally {
          controller.close();
        }
      },
    });

    return new Response(stream, {
      headers: {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache, no-transform',
        Connection: 'keep-alive',
      },
    });
  } catch (error: any) {
    return new Response(JSON.stringify({ error: error?.message || 'Bad request' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }
}
