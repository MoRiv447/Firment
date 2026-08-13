import { describe, expect, it } from 'vitest';
import { accumulateToolCalls, type ToolCallChunk } from '../events';

describe('accumulateToolCalls (event-contract layer)', () => {
  it('concatenates fragmented argument deltas into one complete JSON object', () => {
    const chunks: ToolCallChunk[] = [
      { index: 0, id: 'call_1', name: 'read_file', args: '{"pa' },
      { index: 0, args: 'th":"a.txt"' },
      { index: 0, args: '}' },
    ];
    const calls = accumulateToolCalls(chunks);
    expect(calls).toEqual([
      { id: 'call_1', name: 'read_file', arguments: { path: 'a.txt' } },
    ]);
  });

  it('accumulates multiple parallel tool calls by index', () => {
    const chunks: ToolCallChunk[] = [
      { index: 0, id: 'call_a', name: 'list_dir', args: '{"path":".' },
      { index: 1, id: 'call_b', name: 'grep', args: '{"pattern":"fn ' },
      { index: 0, args: '"}' },
      { index: 1, args: 'main"}' },
    ];
    const calls = accumulateToolCalls(chunks);
    expect(calls).toEqual([
      { id: 'call_a', name: 'list_dir', arguments: { path: '.' } },
      { id: 'call_b', name: 'grep', arguments: { pattern: 'fn main' } },
    ]);
  });

  it('tolerates non-contiguous indices without producing holes', () => {
    const chunks: ToolCallChunk[] = [
      { index: 2, id: 'call_z', name: 'glob', args: '{"pattern":"*.rs"}' },
    ];
    const calls = accumulateToolCalls(chunks);
    expect(calls).toEqual([
      { id: 'call_z', name: 'glob', arguments: { pattern: '*.rs' } },
    ]);
  });

  it('degrades invalid JSON fragments to empty args instead of throwing', () => {
    const chunks: ToolCallChunk[] = [
      { index: 0, id: 'call_x', name: 'web_fetch', args: '{broken json' },
    ];
    expect(() => accumulateToolCalls(chunks)).not.toThrow();
    const calls = accumulateToolCalls(chunks);
    expect(calls[0].name).toBe('web_fetch');
    expect(calls[0].arguments).toEqual({});
  });

  it('drops entries with no function name (id/args-only deltas)', () => {
    const chunks: ToolCallChunk[] = [
      { index: 0, args: '{"orphan":true}' }, // no id/name yet
      { index: 1, id: 'call_y', name: 'write_file', args: '{"path":"x.txt"}' },
    ];
    const calls = accumulateToolCalls(chunks);
    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('write_file');
  });

  it('returns an empty array for an empty chunk stream', () => {
    expect(accumulateToolCalls([])).toEqual([]);
  });
});
