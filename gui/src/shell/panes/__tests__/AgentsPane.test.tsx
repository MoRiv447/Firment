import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { AgentsPane } from '../AgentsPane';
import type { SubagentState } from '../../../lib/turnReducer';

/**
 * The subagent pane.
 *
 * A screenshot cannot verify this one: the browser harness has no event bus, so
 * no subagent ever appears in it. These assertions are the substitute, and the
 * one that matters is the first -- the row is named by the *prompt*, because an
 * id names nothing a person recognises and the tool name is the same word for
 * every delegation.
 */

function agent(over: Partial<SubagentState> = {}): SubagentState {
  return {
    id: 'sub-1',
    label: '研究 STM32F4 EXTI 的寄存器布局',
    depth: 1,
    startedAt: Date.now(),
    steps: [],
    done: false,
    ...over,
  };
}

const step = (seq: number, name: string, status: 'ok' | 'failed' | 'running' = 'ok') => ({
  seq,
  name,
  args: {},
  status,
});

describe('AgentsPane', () => {
  it('names a subagent by the question it was asked', () => {
    render(<AgentsPane subagents={[agent()]} />);
    expect(screen.getByText('研究 STM32F4 EXTI 的寄存器布局')).toBeInTheDocument();
  });

  it('says so when nothing was delegated, instead of showing an empty box', () => {
    render(<AgentsPane subagents={[]} />);
    expect(screen.getByText(/No subagents this turn/)).toBeInTheDocument();
  });

  it('reports a running subagent as running and a finished one as done', () => {
    render(<AgentsPane subagents={[agent({ id: 'a' }), agent({ id: 'b', done: true })]} />);
    expect(screen.getByText('running')).toBeInTheDocument();
    expect(screen.getByText('done')).toBeInTheDocument();
  });

  it('reports a finished subagent with a failed step as failed', () => {
    // "Completed" and "completed successfully" are different things, and the
    // summary a subagent returns does not say which.
    render(
      <AgentsPane
        subagents={[agent({ done: true, steps: [step(1, 'grep', 'failed'), step(2, 'read_file')] })]}
      />,
    );
    expect(screen.getByText('failed')).toBeInTheDocument();
  });

  it('keeps the steps behind a fold, and opens to them', () => {
    const { container } = render(
      <AgentsPane
        subagents={[agent({ done: true, steps: [step(1, 'read_file'), step(2, 'read_file')] })]}
      />,
    );
    // A subagent's twenty calls must not land in the transcript or the pane
    // unfolded; the count is the summary.
    expect(screen.getByText('2 steps')).toBeInTheDocument();
    expect(container.textContent).not.toContain('read_file ×2');
    fireEvent.click(screen.getByText('2 steps'));
    expect(container.textContent).toContain('read_file ×2');
  });

  it('marks a nested delegation so its summary can be weighted', () => {
    render(<AgentsPane subagents={[agent({ depth: 2 })]} />);
    // A depth-2 agent was delegated BY an agent, which changes how much its
    // summary should be trusted.
    expect(screen.getByText('d2')).toBeInTheDocument();
  });
});
