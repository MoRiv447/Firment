import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { LiveRun } from '../LiveRun';
import type { ToolCardState } from '../../types';

/**
 * The live run, as a column of one-line rows.
 *
 * This used to be one folded line per run -- `▸ edit_file · 12s · 4 steps` -- and
 * these cases were written for that: they asserted the fold, the step count and the
 * summary of the run's *shape*. All three are gone, and the reason is written in
 * `LiveRun.tsx`: a card per call is a log and a fold for the whole run hides the
 * step that is failing, so the unit here is the call and the shape of the list is
 * the shape of the work.
 *
 * What the tests still have to hold: one row per call in `seq` order, a row that
 * opens only its own output, and a state per row rather than one verdict for the run.
 */

/** A fixed clock: durations in these assertions are then numbers the test chose. */
const NOW = 1_700_000_000_000;

function tool(over: Partial<ToolCardState> & { seq: number; name: string }): ToolCardState {
  return { args: {}, status: 'ok', ...over };
}

/** The row buttons, in DOM order. The expanded card's head is a `<div>`, so these
    are the only buttons in the tree. */
const rows = () => screen.getAllByRole('button');

describe('LiveRun', () => {
  it('gives every call its own row, in sequence order', () => {
    render(
      <LiveRun
        now={NOW}
        tools={[
          tool({ seq: 3, name: 'edit_file' }),
          tool({ seq: 1, name: 'read_file' }),
          tool({ seq: 2, name: 'build' }),
        ]}
      />,
    );
    // Passed out of order on purpose: `seq` is the turn's own count and the column
    // has to follow it, not the order React happened to be handed.
    expect(rows().map((r) => r.textContent)).toEqual([
      expect.stringContaining('read_file'),
      expect.stringContaining('build'),
      expect.stringContaining('edit_file'),
    ]);
  });

  it('names the tool that is in flight', () => {
    render(
      <LiveRun
        now={NOW}
        tools={[
          tool({ seq: 1, name: 'read_file', status: 'ok' }),
          tool({ seq: 2, name: 'edit_file', status: 'running', startedAt: NOW - 3000 }),
        ]}
      />,
    );
    expect(screen.getByText('edit_file')).toBeInTheDocument();
    expect(screen.getByText('3.0s')).toBeInTheDocument();
  });

  it('keeps the output out of the DOM until its row is opened', () => {
    const { container } = render(
      <LiveRun now={NOW} tools={[tool({ seq: 1, name: 'read_file', summary: 'contents of the file' })]} />,
    );
    // The whole point of a row: a forty-step turn costs forty lines and not forty
    // file contents. The row itself is there either way.
    expect(container.textContent).not.toContain('contents of the file');
    expect(screen.getByText('read_file')).toBeInTheDocument();
  });

  it('opens the card for the row that was pressed, and only that one', () => {
    const { container } = render(
      <LiveRun
        now={NOW}
        tools={[
          tool({ seq: 1, name: 'read_file', summary: 'contents of the file' }),
          tool({ seq: 2, name: 'edit_file', summary: 'two hunks applied' }),
        ]}
      />,
    );
    fireEvent.click(rows()[1]);
    expect(container.textContent).toContain('two hunks applied');
    expect(container.textContent).not.toContain('contents of the file');

    // And pressing the other one closes the first: two open cards push the step you
    // are watching off the screen, which is the one thing this layout is for.
    fireEvent.click(rows()[0]);
    expect(container.textContent).toContain('contents of the file');
    expect(container.textContent).not.toContain('two hunks applied');
  });

  it('marks each row with its own state, not one verdict for the run', () => {
    const { container } = render(
      <LiveRun
        now={NOW}
        tools={[
          tool({ seq: 1, name: 'build', status: 'failed' }),
          tool({ seq: 2, name: 'read_file', status: 'ok' }),
          tool({ seq: 3, name: 'flash', status: 'running' }),
          tool({ seq: 4, name: 'monitor', status: 'unknown' }),
        ]}
      />,
    );
    const states = [...container.querySelectorAll('[data-ui="tool-row"]')].map((r) =>
      r.getAttribute('data-state'),
    );
    // `unknown` is a reopened record and reads as `pending` -- an unrecorded outcome
    // is not a failure. See `lib/steps.ts`.
    expect(states).toEqual(['failed', 'done', 'current', 'pending']);
  });

  it('prints no duration for a step nobody timed', () => {
    // An em dash and not `0.0s`: a zero is a measurement, and this one was never
    // taken.
    render(<LiveRun now={NOW} tools={[tool({ seq: 1, name: 'read_file' })]} />);
    expect(rows()[0]?.textContent).toContain('—');
  });

  it('renders nothing at all when no tool has run', () => {
    const { container } = render(<LiveRun now={NOW} tools={[]} />);
    expect(container.textContent).toBe('');
  });

  it('says where the time went, whenever the clocks are real', () => {
    const now = NOW;
    const { container } = render(
      <LiveRun
        now={NOW}
        turnStartedAt={now - 130_000}
        tools={[
          tool({ seq: 1, name: 'build', startedAt: now - 128_000, endedAt: now - 120_000 }),
          tool({ seq: 2, name: 'flash', startedAt: now - 120_000, endedAt: now, waitedMs: 112_000 }),
        ]}
      />,
    );
    // The two numbers the rows cannot show: the wait was most of the turn, and
    // something ran underneath it.
    expect(container.textContent).toContain('waiting on you');
    expect(container.textContent).toContain('tools');
  });

  it('draws no timeline for a run nobody timed', () => {
    // A reopened transcript: the rows are real, the clocks are not, and a bar built
    // from unknowns would be a claim this app has already refused to make about the
    // per-row durations.
    const { container } = render(
      <LiveRun now={NOW} tools={[tool({ seq: 1, name: 'read_file', summary: 'file contents' })]} />,
    );
    expect(container.querySelector('[data-ui="turn-timeline"]')).toBeNull();
  });
});
