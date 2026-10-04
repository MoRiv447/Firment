import { describe, expect, it } from 'vitest';

/**
 * The monitor's output path, checked where it is wired.
 *
 * `appendMonitorLines` carries the fold and is tested on its own (`monitorLines.test.ts`). What
 * that leaves open is the call site: `App.tsx` is not mountable in this suite (it needs a Tauri
 * runtime and a dozen listeners), so nothing would notice if someone put a
 * `setMonitorLines(...)` back inside `onMonitorOutput` and left the buffer beside it, dead. That
 * edit is exactly the one that returns this to "one full re-render per line of serial output", and
 * it is invisible in a diff because both halves still look correct on their own.
 *
 * So this reads the source, the same way `frontendEvents.test.ts` does, and asks three questions of
 * the handlers themselves. Sources come through Vite's `?raw` glob because this project has no
 * `@types/node`.
 */
const SOURCES = import.meta.glob('../../**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

const app = (): string => {
  const source = Object.entries(SOURCES)
    .filter(([path]) => path.endsWith('/App.tsx'))
    .map(([, text]) => text)[0];
  if (!source) throw new Error('App.tsx was not read; the glob no longer covers it');
  return source;
};

/** The text from `from` up to the next `until` — one handler's body, no more. */
const between = (source: string, from: string, until: string): string => {
  const start = source.indexOf(from);
  expect(start, `${from} is not in App.tsx, so this gate is reading nothing`).toBeGreaterThanOrEqual(
    0,
  );
  const stop = source.indexOf(until, start);
  expect(stop, `${from} is not followed by ${JSON.stringify(until)} — the shape moved`).toBeGreaterThan(
    start,
  );
  const slice = source.slice(start, stop);
  expect(slice.length, `${from} came back too short to be a handler`).toBeGreaterThan(40);
  return slice;
};

describe('the monitor output path', () => {
  it('buffers each line instead of writing state once per line', () => {
    const handler = between(app(), 'onMonitorOutput(', 'onMonitorExited(');
    expect(handler, 'a per-line setState is what this whole path replaced').not.toContain(
      'setMonitorLines',
    );
    expect(handler).toContain('monitorBuffer.push(line)');
    expect(handler, 'one timer for the burst, not one per line').toContain(
      'setTimeout(flushMonitor, 50)',
    );
  });

  it('folds through the tested helper', () => {
    const flush = between(app(), 'const flushMonitor = ', 'unlisteners.push(');
    expect(flush).toContain('appendMonitorLines(prev');
  });

  it('sends the exit banner through the same queue, so it cannot overtake what it follows', () => {
    const handler = between(app(), 'onMonitorExited(', '\n    );');
    expect(handler).toContain('monitorBuffer.push');
    expect(handler, 'flushing is what puts the banner after the lines before it').toContain(
      'flushMonitor()',
    );
    expect(handler).not.toContain('setMonitorLines');
  });
});
