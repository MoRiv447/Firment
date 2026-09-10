import { afterEach, describe, expect, it, vi } from 'vitest';

import { webFetch } from '../tools/web';

/**
 * FIR-002's other half: the loop refusing to START a queued tool is useless
 * unless the outbound request already in flight is cancelled too. web_fetch
 * has a 20s deadline, so a disconnected client would otherwise keep a socket
 * open for 20s per remaining call. `startDeadline` composes the caller signal
 * with its own timer; these tests pin that composition down offline (192.0.2.0/24
 * is the documentation range — public enough to pass the SSRF guard, never
 * contacted because fetch is stubbed).
 */
const PUBLIC_URL = 'http://192.0.2.10/datasheet.html';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('web_fetch caller-signal wiring', () => {
  it('aborts the in-flight request when the caller disconnects', async () => {
    const caller = new AbortController();
    let fetchSignal: AbortSignal | undefined;

    vi.stubGlobal(
      'fetch',
      (_url: any, init: any) =>
        new Promise((_resolve, reject) => {
          fetchSignal = init.signal;
          // Disconnect at the moment the request is issued, like a closed tab.
          caller.abort();
          if (init.signal.aborted) reject(new Error('aborted'));
          else init.signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true });
        })
    );

    await expect(webFetch(PUBLIC_URL, caller.signal)).rejects.toThrow(/Fetch failed/);
    expect(fetchSignal, 'fetch must receive a signal').toBeDefined();
    expect(fetchSignal!.aborted).toBe(true);
  });

  it('hands fetch an already-aborted signal when the client went away first', async () => {
    const caller = new AbortController();
    caller.abort();
    let fetchSignal: AbortSignal | undefined;

    vi.stubGlobal(
      'fetch',
      (_url: any, init: any) =>
        new Promise((_resolve, reject) => {
          fetchSignal = init.signal;
          reject(new Error('aborted'));
        })
    );

    await expect(webFetch(PUBLIC_URL, caller.signal)).rejects.toThrow(/Fetch failed/);
    expect(fetchSignal!.aborted).toBe(true);
  });

  it('still runs without a caller signal (deadline-only path)', async () => {
    let captured: AbortSignal | undefined;
    vi.stubGlobal('fetch', async (_url: any, init: any) => {
      captured = init.signal;
      return { ok: true, status: 200, headers: { get: () => 'text/html' }, text: async () => '<html><body><p>ok</p></body></html>' };
    });

    const text = await webFetch(PUBLIC_URL);
    expect(text).toContain('ok');
    expect(captured?.aborted).toBe(false);
  });
});
