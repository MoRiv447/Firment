import { describe, expect, it } from 'vitest';

import { MONITOR_LINE_CAP, appendMonitorLines } from '../monitorLines';
import type { MonitorLine } from '../../types';

const line = (port: string, text: string): MonitorLine => ({ port, kind: 'stdout', line: text });

const many = (port: string, count: number, from = 0): MonitorLine[] =>
  Array.from({ length: count }, (_, i) => line(port, `${from + i}`));

describe('appendMonitorLines', () => {
  it('folds a burst into one update, per port, in arrival order', () => {
    const prev = { COM3: [line('COM3', 'boot')] };
    const next = appendMonitorLines(prev, [
      line('COM3', 'a'),
      line('COM14', 'x'),
      line('COM3', 'b'),
      line('COM14', 'y'),
    ]);
    expect(next.COM3.map((l) => l.line)).toEqual(['boot', 'a', 'b']);
    expect(next.COM14.map((l) => l.line)).toEqual(['x', 'y']);
    // The order inside the batch is the order on screen. Sorting by port instead would interleave
    // two monitors' text into whichever one happened to be alphabetically first.
    expect(next.COM3[1].line, 'a line from the other port came between these two').toBe('a');
  });

  it('keeps the tail of a flood, not its head, and only the cap of it', () => {
    const next = appendMonitorLines({}, many('COM3', MONITOR_LINE_CAP + 5));
    expect(next.COM3).toHaveLength(MONITOR_LINE_CAP);
    expect(next.COM3[0].line).toBe('5');
    expect(next.COM3[MONITOR_LINE_CAP - 1].line).toBe(String(MONITOR_LINE_CAP + 4));
  });

  it('applies the cap across flushes, not just within one', () => {
    // Coalescing means the array grows in steps now. A cap that only looked inside the batch would
    // let a monitor that streams steadily for a minute hold every line it ever printed.
    let state = appendMonitorLines({}, many('COM3', 1_500));
    state = appendMonitorLines(state, many('COM3', 1_500, 1_500));
    expect(state.COM3).toHaveLength(MONITOR_LINE_CAP);
    expect(state.COM3[0].line).toBe('1000');
  });

  it('hands back the same map when the batch is empty', () => {
    const prev = { COM3: many('COM3', 3) };
    expect(appendMonitorLines(prev, [])).toBe(prev);
  });

  it('does not touch the arrays it was given', () => {
    const prev = { COM3: many('COM3', 3) };
    const before = [...prev.COM3];
    const next = appendMonitorLines(prev, many('COM3', 2, 3));
    expect(prev.COM3).toEqual(before);
    expect(next).not.toBe(prev);
    expect(next.COM3).not.toBe(prev.COM3);
  });
});
