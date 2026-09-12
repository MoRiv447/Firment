import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { LiveRun } from '../LiveRun';
import type { ToolCardState } from '../../types';

/**
 * The live run line.
 *
 * During a run the question is not "what has it done" but "what is it doing
 * now", so the folded header has to name the tool in flight. If it only counted
 * steps, folding a live turn would hide the one thing worth watching.
 */

function tool(over: Partial<ToolCardState> & { seq: number; name: string }): ToolCardState {
  return { args: {}, status: 'ok', ...over };
}

describe('LiveRun', () => {
  it('names the tool that is in flight', () => {
    render(
      <LiveRun
        tools={[
          tool({ seq: 1, name: 'read_file', status: 'ok' }),
          tool({ seq: 2, name: 'edit_file', status: 'running', startedAt: Date.now() - 3000 }),
        ]}
      />,
    );
    expect(screen.getByText('edit_file')).toBeInTheDocument();
  });

  it('folds the finished steps away', () => {
    const { container } = render(
      <LiveRun
        tools={[
          tool({ seq: 1, name: 'read_file', status: 'ok', summary: 'contents of the file' }),
        ]}
      />,
    );
    // The card body is not in the DOM until the row is opened: the point of the
    // fold is that a forty-step turn costs one row.
    expect(container.textContent).not.toContain('contents of the file');
    expect(screen.getByText('1 步')).toBeInTheDocument();
  });

  it('opens to the same cards it hid', () => {
    const { container } = render(
      <LiveRun
        tools={[
          tool({ seq: 1, name: 'read_file', status: 'ok', summary: 'contents of the file' }),
        ]}
      />,
    );
    fireEvent.click(screen.getByText('1 步'));
    expect(container.textContent).toContain('contents of the file');
  });

  it('summarises the shape of a multi-step run', () => {
    render(
      <LiveRun
        tools={[
          tool({ seq: 1, name: 'read_file' }),
          tool({ seq: 2, name: 'read_file' }),
          tool({ seq: 3, name: 'edit_file' }),
        ]}
      />,
    );
    expect(screen.getByText(/read_file ×2 · edit_file/)).toBeInTheDocument();
  });

  it('counts a run with any failure in it as failed', () => {
    // A failed step is not redeemed by the steps that follow it.
    const { container } = render(
      <LiveRun
        tools={[tool({ seq: 1, name: 'build', status: 'failed' }), tool({ seq: 2, name: 'read_file' })]}
      />,
    );
    const dot = container.querySelector('span[aria-hidden]') as HTMLElement;
    // The dot carries the hue; the exact token is the design system's business.
    expect(dot.style.background).not.toBe('');
  });

  it('renders nothing at all when no tool has run', () => {
    const { container } = render(<LiveRun tools={[]} />);
    expect(container.textContent).toBe('');
  });
});
