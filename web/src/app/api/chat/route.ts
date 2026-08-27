import { NextRequest } from 'next/server';
import { runAgentTurn } from '@/lib/agent';

export const dynamic = 'force-dynamic';

/**
 * Bearer-token gate for the chat endpoint.
 *
 * - If FIRMENT_WEB_API_KEY is set, requests must present
 *   `Authorization: Bearer <key>` (constant-time compare).
 * - If it is not set, the endpoint only works in non-production
 *   (local dev); a production deployment without a key refuses to
 *   serve, so a public Vercel app can't burn LLM quota anonymously.
 */
function authorize(request: NextRequest): string | null {
  const apiKey = process.env.FIRMENT_WEB_API_KEY;
  if (!apiKey) {
    return process.env.NODE_ENV === 'production'
      ? 'FIRMENT_WEB_API_KEY is not configured on this deployment'
      : null;
  }
  const auth = request.headers.get('authorization') ?? '';
  const presented = auth.startsWith('Bearer ') ? auth.slice(7) : '';
  if (!presented || presented.length !== apiKey.length) {
    return 'Invalid or missing API key';
  }
  // Constant-time compare to avoid timing attacks.
  let diff = 0;
  for (let i = 0; i < apiKey.length; i++) {
    diff |= presented.charCodeAt(i) ^ apiKey.charCodeAt(i);
  }
  return diff === 0 ? null : 'Invalid API key';
}

export async function POST(request: NextRequest) {
  const authError = authorize(request);
  if (authError) {
    return new Response(JSON.stringify({ error: authError }), {
      status: 401,
      headers: { 'Content-Type': 'application/json' },
    });
  }
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
    // Client disconnects must stop the whole agent loop (LLM streaming +
    // further tool executions), not let it burn tokens against a dead pipe.
    const abort = new AbortController();
    if (request.signal.aborted) {
      abort.abort();
    } else {
      request.signal.addEventListener('abort', () => abort.abort(), { once: true });
    }

    const stream = new ReadableStream({
      async start(controller) {
        // After a disconnect every enqueue throws; abort instead of letting
        // the TypeError unwind through the agent loop.
        const send = (obj: any) => {
          try {
            controller.enqueue(encoder.encode(`data: ${JSON.stringify(obj)}\n\n`));
          } catch {
            abort.abort();
          }
        };

        try {
          const result = await runAgentTurn(
            messages,
            userInput,
            config || {},
            (event) => send(event),
            abort.signal
          );
          send({ type: 'done', finalText: result.finalText, newMessages: result.newMessages });
        } catch (error: any) {
          if (!abort.signal.aborted) {
            console.error('Chat error:', error);
            send({ type: 'error', error: error?.message || 'Internal server error' });
          }
        } finally {
          try {
            controller.close();
          } catch {
            // already cancelled by the disconnect
          }
        }
      },
      cancel() {
        abort.abort();
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
