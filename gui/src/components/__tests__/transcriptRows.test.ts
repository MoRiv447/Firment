import { describe, expect, it } from 'vitest';
import { groupTranscript } from '../MessageList';
import type { ChatMessage } from '../../types';

/**
 * The transcript's unit is a *run of tool work*, not a message.
 *
 * A turn that read four files and made two edits used to render twelve rows --
 * six call cards and six result cards -- and none of them is what anybody reads.
 * Folding them to one line is the change; these assertions are about where the
 * folds are allowed to happen, because a fold in the wrong place hides prose.
 */

const user = (content: string): ChatMessage => ({ role: 'user', content });
const prose = (content: string): ChatMessage => ({ role: 'assistant', content });
const call = (id: string, name: string): ChatMessage => ({
  role: 'assistant',
  content: '',
  tool_calls: [{ id, name, arguments: { path: 'src/foc/current.c' } }],
});
const result = (id: string, content: string): ChatMessage => ({
  role: 'tool',
  tool_call_id: id,
  name: 'read_file',
  content,
});

describe('groupTranscript', () => {
  it('folds a run of calls and results into one row', () => {
    const rows = groupTranscript([
      user('fix the loop'),
      call('t1', 'read_file'),
      result('t1', 'a'),
      call('t2', 'edit_file'),
      result('t2', 'b'),
    ]);
    expect(rows.map((r) => r.kind)).toEqual(['single', 'run']);
    const run = rows[1];
    expect(run.kind === 'run' && run.messages).toHaveLength(4);
  });

  it('does not fold prose into a run', () => {
    // The prose is the thing the reader is looking for. A run that swallowed it
    // would hide the answer behind a disclosure triangle.
    const rows = groupTranscript([
      call('t1', 'read_file'),
      result('t1', 'a'),
      prose('## 定位\n\n采样点和更新点重合。'),
      call('t2', 'edit_file'),
      result('t2', 'b'),
    ]);
    expect(rows.map((r) => r.kind)).toEqual(['run', 'single', 'run']);
  });

  it('keeps an assistant message that has both text and calls as prose', () => {
    const rows = groupTranscript([
      { role: 'assistant', content: 'Let me look.', tool_calls: [{ id: 't1', name: 'read_file', arguments: {} }] },
    ]);
    expect(rows.map((r) => r.kind)).toEqual(['single']);
  });

  it('leaves a transcript with no tool work completely alone', () => {
    const rows = groupTranscript([user('hi'), prose('hello')]);
    expect(rows.map((r) => r.kind)).toEqual(['single', 'single']);
  });

  it('folds a run that starts the transcript', () => {
    const rows = groupTranscript([call('t1', 'read_file'), result('t1', 'a')]);
    expect(rows.map((r) => r.kind)).toEqual(['run']);
  });

  it('gives every row a key that does not depend on position alone', () => {
    // Index-only keys made expansion state migrate to the wrong card when the
    // optimistic-append → transcript-refresh cycle shifted rows.
    const rows = groupTranscript([user('a'), call('t1', 'read_file'), user('b')]);
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
    expect(rows[1].key).toContain('t1');
  });

  it('handles an empty transcript', () => {
    expect(groupTranscript([])).toEqual([]);
  });
});
