import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

import pkg from '../../../package.json';
import { SessionSidebar } from '../SessionSidebar';
import type { SessionSummaryDto } from '../../types';

/**
 * The rail's structure, not its colours.
 *
 * What is worth pinning is the shape of the tree and the fact that no colour is
 * assembled in JS any more:
 *
 * * Branches nest under their mainline, and a session whose parent is gone is
 *   hoisted rather than dropped -- a row that renders nowhere is a session that
 *   cannot be deleted from this window.
 * * One category chip per row, guaranteed by `kindOf` being an `if` chain rather
 *   than three `&&`s in markup.
 * * The workbench and delete controls sit BESIDE the row's own button, so
 *   neither has to stop the row's click: opening the workbench or answering a
 *   question about deleting a row leaves the transcript alone.
 */

type RailProps = ComponentProps<typeof SessionSidebar>;

const session = (over: Partial<SessionSummaryDto> & { id: string }): SessionSummaryDto => ({
  kind: 'normal',
  parent_session: null,
  preview: `preview ${over.id}`,
  model: 'claude-sonnet-4-5',
  cwd: 'C:\\work\\proj',
  updated_at: 1_700_000_000,
  ...over,
});

const branch = (id: string, parent: string, preview: string, updated_at: number) =>
  session({ id, kind: 'branch', parent_session: parent, preview, updated_at });

const railHandlers = () => ({
  onWorkCwd: vi.fn(),
  onSelect: vi.fn(),
  onNew: vi.fn(),
  onDelete: vi.fn(),
  onOpenWorkbench: vi.fn(),
});

function renderRail(over: Partial<RailProps> = {}) {
  const handlers = railHandlers();
  const props: RailProps = {
    sessions: [],
    currentId: null,
    workCwd: 'C:\\work',
    ...handlers,
    ...over,
  };
  const view = render(<SessionSidebar {...props} />);
  return {
    ...view,
    handlers,
    again: (next: Partial<RailProps>) => view.rerender(<SessionSidebar {...props} {...next} />),
  };
}

/** Every row's own button, in the order the rail printed them. */
const rowsIn = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-ui="session-row"]'));

/** The first titled span in a row is its session title; the model span is second. */
const titles = (container: HTMLElement) =>
  rowsIn(container).map((row) => row.querySelector('span[title]')?.textContent);

/** The row's indent is a custom property, so ask for it by name rather than parsing `style`. */
const depthOf = (row: Element) =>
  (row.closest('li') as HTMLElement | null)?.style.getPropertyValue('--depth').trim();
const kindOf = (row: Element) => {
  const chip = row.querySelector('[data-ui="chip"]');
  return { text: chip?.textContent, status: chip?.getAttribute('data-status') };
};

describe('session tree', () => {
  it('nests branches under their mainline, newest root first', () => {
    const { container } = renderRail({
      sessions: [
        branch('b1', 'm', 'first fork', 20),
        session({ id: 'm', kind: 'mainline', preview: 'mainline root', updated_at: 30 }),
        branch('b2', 'm', 'later fork', 40),
        session({ id: 'old', preview: 'stale chat', updated_at: 10 }),
      ],
    });
    const rows = rowsIn(container);
    expect(titles(container)).toEqual(['mainline root', 'first fork', 'later fork', 'stale chat']);
    expect(rows.map(depthOf)).toEqual(['0', '1', '1', '0']);
  });

  it('hoists a branch whose parent is gone, so it can still be deleted', () => {
    const { container } = renderRail({
      sessions: [branch('orphan', 'gone', 'the only row', 5)],
    });
    expect(titles(container)).toEqual(['the only row']);
    expect(rowsIn(container).map(kindOf)).toEqual([{ text: '↳ BRANCH', status: 'neutral' }]);
  });

  it('does not become its own child forever', () => {
    const { container } = renderRail({ sessions: [branch('loop', 'loop', 'self-parented', 5)] });
    expect(container.querySelectorAll('[data-ui="session-row"]')).toHaveLength(1);
  });

  it('gives every row exactly one category chip', () => {
    const { container } = renderRail({
      sessions: [
        session({ id: 'm', kind: 'mainline' }),
        session({ id: 'n', kind: 'normal' }),
        branch('b', 'm', 'a fork', 5),
        session({ id: 'x', kind: 'unknown-to-the-ui' }),
      ],
    });
    const rows = rowsIn(container);
    // A `kind` the kernel never sends still has to read as something, and `x` is
    // the case that would print no chip at all if the markup were three `&&`s.
    // The walk is depth-first, so `b` sits right below its parent `m` instead of
    // keeping its place in the input array.
    expect(rows.map((row) => row.querySelectorAll('[data-ui="chip"]').length)).toEqual([
      1, 1, 1, 1,
    ]);
    expect(rows.map(kindOf)).toEqual([
      { text: 'MAINLINE', status: 'ok' },
      { text: '↳ BRANCH', status: 'neutral' },
      { text: 'NORMAL', status: 'neutral' },
      { text: 'NORMAL', status: 'neutral' },
    ]);
  });
});

describe('row state', () => {
  it('marks the current session and only that one', () => {
    const { container } = renderRail({
      sessions: [session({ id: 'a' }), session({ id: 'b' })],
      currentId: 'b',
    });
    const rows = rowsIn(container);
    expect(rows[0]?.closest('li')).not.toHaveAttribute('data-selected');
    expect(rows[1]).toHaveAttribute('aria-current', 'true');
    expect(rows[1]?.closest('li')).toHaveAttribute('data-selected', 'true');
  });

  it('paints nothing in JS, so a selected row cannot put green ink on green', () => {
    const { container } = renderRail({
      sessions: [session({ id: 'a' }), session({ id: 'b' })],
      currentId: 'b',
      runningIds: new Set(['a', 'b']),
    });
    // The rail used to rebuild every chip's `background` and `color` for a
    // selected row. `Chip` pairs its own opaque fill and ink now, so a chip that
    // carries an inline style is the old bug coming back.
    expect(container.querySelectorAll('[data-ui="chip"][style]')).toHaveLength(0);
  });

  it('reports a background turn as running, and the rest as not', () => {
    const { container } = renderRail({
      sessions: [session({ id: 'a' }), session({ id: 'b' })],
      runningIds: new Set(['b']),
    });
    expect(screen.getAllByText('running')).toHaveLength(1);
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(1);
  });

  it('selects the session when the row itself is clicked', () => {
    const { handlers, container } = renderRail({ sessions: [session({ id: 'a' })] });
    fireEvent.click(container.querySelector('[data-ui="session-row"]') as Element);
    expect(handlers.onSelect).toHaveBeenCalledOnce();
  });
});

describe('header', () => {
  it('starts an agent session, or a plan-mode one', () => {
    const { handlers } = renderRail();
    fireEvent.click(screen.getByRole('button', { name: 'New' }));
    fireEvent.click(screen.getByRole('button', { name: 'New plan-mode session' }));
    expect(handlers.onNew.mock.calls).toEqual([['agent'], ['plan']]);
  });

  it('names the working directory in a visible label, and stays controlled', () => {
    // The label is the accessible name now: `Field` wires `htmlFor` to the
    // control, so an `aria-label` beside it would only be a second name to
    // disagree with.
    const { handlers } = renderRail();
    const field = screen.getByRole('textbox', { name: 'new sessions in' });
    fireEvent.change(field, { target: { value: 'D:\\old\\proj' } });
    expect(handlers.onWorkCwd).toHaveBeenCalledWith('D:\\old\\proj');
    // Controlled: what the rail shows is the caller's value, not what was typed.
    expect(field).toHaveValue('C:\\work');
  });

  it('names a session that has said nothing yet', () => {
    // Its title slot is a preview of the conversation, so an empty session used to
    // render a row with no name at all -- chip, model and time, which reads as a
    // nameless thing rather than as a new one.
    renderRail({ sessions: [session({ id: 'a', preview: '' })] });
    expect(screen.getByText('New session')).toBeInTheDocument();
  });

  it('counts the sessions it is holding', () => {
    const { again } = renderRail({ sessions: [session({ id: 'a' })] });
    expect(screen.getByText('1 session')).toBeInTheDocument();
    again({ sessions: [session({ id: 'a' }), session({ id: 'b' })] });
    expect(screen.getByText('2 sessions')).toBeInTheDocument();
  });

  it('carries the build the window was made from', () => {
    const { container } = renderRail();
    expect(container).toHaveTextContent(`v${pkg.version}`);
  });

  it('says what would fill an empty rail', () => {
    renderRail();
    expect(screen.getByText('No sessions yet')).toBeInTheDocument();
    expect(screen.getByText(/New above starts one/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'New' })).toBeInTheDocument();
  });
});

describe('row controls', () => {
  const root = session({ id: 'm', kind: 'mainline', cwd: 'D:\\firm\\bot' });
  const kids: SessionSummaryDto[] = [root, branch('b', 'm', 'a fork', 5)];

  const workbenchButtons = () =>
    screen.queryAllByRole('button', { name: "Open this project's workbench" });

  it('offers the workbench only on a mainline that has branches', () => {
    const { again } = renderRail({ sessions: kids });
    expect(workbenchButtons()).toHaveLength(1);
    again({ sessions: [root] });
    expect(workbenchButtons()).toHaveLength(0);
  });

  it('opens the workbench without selecting the session underneath it', () => {
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(screen.getByRole('button', { name: "Open this project's workbench" }));
    expect(handlers.onOpenWorkbench).toHaveBeenCalledWith('D:\\firm\\bot');
    expect(handlers.onSelect).not.toHaveBeenCalled();
  });

  it('asks beside the row it is about, and only answers on Delete', () => {
    const { handlers } = renderRail({ sessions: kids });
    const trash = screen.getAllByRole('button', { name: 'Delete this session' });
    fireEvent.click(trash[1]);

    const panel = screen.getByRole('dialog', { name: 'Delete this session?' });
    expect(panel).toHaveTextContent('a fork');
    expect(handlers.onDelete).not.toHaveBeenCalled();

    fireEvent.click(within(panel).getByRole('button', { name: 'Delete' }));
    expect(handlers.onDelete).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(handlers.onSelect).not.toHaveBeenCalled();
  });

  it('cancels without deleting', () => {
    const { handlers } = renderRail({ sessions: [session({ id: 'a' })] });
    fireEvent.click(screen.getByRole('button', { name: 'Delete this session' }));
    const panel = screen.getByRole('dialog', { name: 'Delete this session?' });
    fireEvent.click(within(panel).getByRole('button', { name: 'Cancel' }));
    expect(handlers.onDelete).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
