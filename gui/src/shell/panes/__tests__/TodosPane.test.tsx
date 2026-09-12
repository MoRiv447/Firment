import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { TodosPane, todoSummary } from '../TodosPane';
import type { TodoDto } from '../../../types';

/**
 * The agent's todo list.
 *
 * It is read-only on purpose -- the agent owns the writes, and the tool saves
 * atomically, so a second writer would race it. What the pane has to get right
 * is telling you *where the agent is*, which is the first unfinished item, and
 * not erasing the record of what it planned.
 */

const todo = (text: string, done = false): TodoDto => ({ text, done });

describe('TodosPane', () => {
  it('marks the first unfinished item as where the agent is now', () => {
    const { container } = render(
      <TodosPane
        todos={[todo('读电流环', true), todo('改 Ts'), todo('跑回归')]}
        loading={false}
      />,
    );
    // Everything above it is accounted for; marking it is what makes the list
    // readable at a glance instead of a list you have to scan.
    const marks = [...container.querySelectorAll('li span[aria-hidden]')].map((s) => s.textContent);
    expect(marks).toEqual(['✓', '▸', '○']);
  });

  it('strikes through done items instead of removing them', () => {
    // The list is the record of the plan. A plan that erases itself cannot be
    // checked against what actually happened.
    render(<TodosPane todos={[todo('读电流环', true), todo('改 Ts')]} loading={false} />);
    // The decoration is on the row, not on the text span `getByText` returns.
    expect(screen.getByText('读电流环').closest('li')).toHaveStyle({
      textDecoration: 'line-through',
    });
    expect(screen.getByText('改 Ts').closest('li')).not.toHaveStyle({
      textDecoration: 'line-through',
    });
  });

  it('counts progress', () => {
    render(
      <TodosPane todos={[todo('a', true), todo('b', true), todo('c')]} loading={false} />,
    );
    expect(screen.getByText('2/3')).toBeInTheDocument();
  });

  it('marks nothing as current when everything is done', () => {
    const { container } = render(
      <TodosPane todos={[todo('a', true), todo('b', true)]} loading={false} />,
    );
    const marks = [...container.querySelectorAll('li span[aria-hidden]')].map((s) => s.textContent);
    expect(marks).toEqual(['✓', '✓']);
  });

  it('distinguishes an empty list from one still loading', () => {
    const { unmount } = render(<TodosPane todos={[]} loading />);
    expect(screen.getByText('Loading…')).toBeInTheDocument();
    unmount();
    render(<TodosPane todos={[]} loading={false} />);
    expect(screen.getByText(/No todos in this session yet/)).toBeInTheDocument();
  });
});

describe('todoSummary', () => {
  it('is nothing at all when there is no list', () => {
    // The status bar shows nothing rather than "0/0", which would read as a
    // failed task.
    expect(todoSummary([])).toBeNull();
  });

  it('is done/total otherwise', () => {
    expect(todoSummary([todo('a', true), todo('b')])).toBe('1/2');
  });
});
