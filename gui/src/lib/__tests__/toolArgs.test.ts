import { describe, expect, it } from 'vitest';
import { describeArgs } from '../toolArgs';

/**
 * The argument line on a tool card.
 *
 * `{"path":"src/foc/current.c"}` was what the transcript showed for a file
 * read. These assertions are about the shape of the answer, not about any one
 * tool: name the subject, keep the rest short, and never throw — an agent can
 * send a string where an object was expected, and that must not take the
 * transcript render down with it.
 */

describe('describeArgs', () => {
  it('shows the subject of a call rather than the field name', () => {
    expect(describeArgs({ path: 'src/foc/current.c' })).toBe('src/foc/current.c');
    expect(describeArgs({ url: 'https://example.com/an.pdf' })).toBe('https://example.com/an.pdf');
  });

  it('keeps the remaining keys, compactly', () => {
    expect(describeArgs({ pattern: 'dma_start', glob: '*.c' })).toBe('dma_start · glob=*.c');
  });

  it('quotes a value with spaces so the fields stay distinguishable', () => {
    expect(describeArgs({ query: 'stm32 exti' })).toBe('"stm32 exti"');
  });

  it('prefers the most specific subject when a tool has several', () => {
    // edit_file carries a path and two bodies. The path is what a reader scans
    // for; the bodies must not push it off the line.
    const line = describeArgs({
      path: 'src/main.c',
      old_text: 'a very long body that would otherwise dominate the line',
      new_text: 'another very long body',
    });
    expect(line.startsWith('src/main.c · ')).toBe(true);
  });

  it('falls back to key=value pairs when nothing names a subject', () => {
    expect(describeArgs({ baud: 115200, timeout: 30 })).toBe('baud=115200 · timeout=30');
  });

  it('renders arrays and nested objects without dumping them', () => {
    expect(describeArgs({ path: 'a.c', edits: [{ a: 1 }, { b: 2 }] })).toBe(
      'a.c · edits=[{1 keys}, {1 keys}]',
    );
  });

  it('truncates at the limit and says so', () => {
    const line = describeArgs({ path: 'x'.repeat(200) }, 40);
    expect(line).toHaveLength(41);
    expect(line.endsWith('…')).toBe(true);
  });

  it('passes a bare string through, because some backends stream arguments', () => {
    expect(describeArgs('{"partial":')).toBe('{"partial":');
  });

  it('returns nothing for nothing, instead of "null" or "{}"', () => {
    expect(describeArgs(undefined)).toBe('');
    expect(describeArgs(null)).toBe('');
    expect(describeArgs({})).toBe('');
  });

  it('drops keys whose value is undefined rather than printing "undefined"', () => {
    expect(describeArgs({ path: 'a.c', limit: undefined })).toBe('a.c');
  });
});
