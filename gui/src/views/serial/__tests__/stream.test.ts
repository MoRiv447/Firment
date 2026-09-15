import { describe, expect, it } from 'vitest';
import { toStream } from '../stream';
import type { MonitorLine } from '../../../types';

/**
 * The chunk-to-line rule.
 *
 * It came out of the component because it was the only part of that view that
 * could be wrong without looking wrong: a terminal that glues chunks the wrong way
 * still renders, it just reads like a list of fragments.
 */

const line = (text: string, kind: MonitorLine['kind'] = 'stdout'): MonitorLine =>
  ({ line: text, kind }) as MonitorLine;

describe('toStream', () => {
  it('keeps whole lines apart', () => {
    expect(toStream([line('one\n'), line('two\n')])).toEqual([
      { text: 'one', stderr: false },
      { text: 'two', stderr: false },
    ]);
  });

  it('glues a chunk that ends mid-word onto the one before it', () => {
    // A serial read hands over whatever the port had. Without this, "Hel" and
    // "lo\n" would be two lines.
    expect(toStream([line('Hel'), line('lo\n')])).toEqual([{ text: 'Hello', stderr: false }]);
  });

  it('starts a new line after one that ended with a newline', () => {
    // The trap: without the boundary, everything after the first newline glues
    // onto the same line forever.
    expect(toStream([line('one\n'), line('two'), line('three\n')])).toEqual([
      { text: 'one', stderr: false },
      { text: 'twothree', stderr: false },
    ]);
  });

  it('only glues stderr onto stderr', () => {
    // A warning in the middle of a sentence keeps its own colour.
    expect(toStream([line('partial', 'stdout'), line('warning', 'stderr')])).toEqual([
      { text: 'partial', stderr: false },
      { text: 'warning', stderr: true },
    ]);
  });

  it('leaves a trailing newline off the rendered text', () => {
    // The newline is a boundary, not a character to print.
    expect(toStream([line('\n')])).toEqual([{ text: '', stderr: false }]);
  });

  it('is empty for nothing', () => {
    expect(toStream([])).toEqual([]);
  });
});
