// The session rail's own tests, rewritten for the row that has no chrome.
//
// Three assumptions died with the restructure and each of them was load-bearing for
// more than one case: the row's first `span[title]` was its name (there is one span
// now and no separate model span), every row carried exactly one kind chip (there is
// no chip), and each row had a bin button beside it (the actions are behind a menu).
// What replaces them is the name as text and the menu as a thing you open.
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';

import pkg from '../../../package.json';
import { SessionSidebar } from '../SessionSidebar';
import type { SessionSummaryDto } from '../../types';

type RailProps = Parameters<typeof SessionSidebar>[0];

const session = (over: Partial<SessionSummaryDto> = {}): SessionSummaryDto => ({
  id: 'a',
  kind: 'normal',
  parent_session: null,
  preview: `preview ${over.id ?? 'a'}`,
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
  onRename: vi.fn(),
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

/**
 * What each row says.
 *
 * The row is a name and nothing else now -- the running mark is a dot with no text,
 * and it is `aria-hidden` -- so the button's text content *is* the name. That is a
 * better handle than the class it used to be found by: a rename or a new badge cannot
 * silently change what this returns.
 */
const titles = (container: HTMLElement) => rowsIn(container).map((row) => row.textContent);

/** The row's indent is a custom property, so ask for it by name rather than parsing `style`. */
const depthOf = (row: Element) =>
  (row.closest('li') as HTMLElement | null)?.style.getPropertyValue('--depth').trim();

/** The options button names its own row, which is also how a test finds the right one. */
const moreFor = (name: string) => screen.getByRole('button', { name: `Options for ${name}` });

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
    // A branch is told apart by its indent and by the rail's own tree, not by a chip:
    // the row carries no category at all any more.
    expect(container.querySelectorAll('[data-ui="chip"]')).toHaveLength(0);
  });

  it('does not become its own child forever', () => {
    const { container } = renderRail({ sessions: [branch('loop', 'loop', 'self-parented', 5)] });
    expect(container.querySelectorAll('[data-ui="session-row"]')).toHaveLength(1);
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
    // The rail used to rebuild every chip's `background` and `color` for a selected
    // row. Nothing in the rail is painted from a `style` attribute now.
    expect(container.querySelectorAll('[style*="background"]')).toHaveLength(0);
  });

  it('reports a background turn with a mark, and the rest without one', () => {
    // The word "running" is gone with the meta line the row used to carry. The mark
    // is what is left of that signal, and it is the only thing on a row that is not
    // the name -- which is why this case still exists.
    const { container } = renderRail({
      sessions: [session({ id: 'a' }), session({ id: 'b' })],
      runningIds: new Set(['b']),
    });
    const marks = container.querySelectorAll('[data-ui="status-dot"]');
    expect(marks).toHaveLength(1);
    expect(marks[0]).toHaveAttribute('data-status', 'running');
    expect(titles(container)).toEqual(['preview a', 'preview b']);
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
    const { handlers } = renderRail();
    const field = screen.getByRole('textbox', { name: 'new sessions in' });
    fireEvent.change(field, { target: { value: 'D:\\old\\proj' } });
    expect(handlers.onWorkCwd).toHaveBeenCalledWith('D:\\old\\proj');
    expect(field).toHaveValue('C:\\work');
  });

  it('names a session that has said nothing yet', () => {
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

describe('row actions', () => {
  const root = session({ id: 'm', kind: 'mainline', preview: 'a mainline', cwd: 'D:\\firm\\bot' });
  const kids: SessionSummaryDto[] = [root, branch('b', 'm', 'a fork', 5)];

  /** Open a row's menu and hand back its panel. */
  const openMenu = (name: string) => {
    // Close first: the trigger toggles, and a re-render keeps the open state, so the
    // second press in one of these cases would shut the menu rather than open it.
    const open = screen.queryByRole('menu');
    if (open) fireEvent.keyDown(open, { key: 'Escape' });
    fireEvent.click(moreFor(name));
    return screen.getByRole('menu');
  };

  it('keeps the destructive action off the surface', () => {
    // It was a bin icon on every row -- the one irreversible thing in the rail, at the
    // weight of the row's own name, on a surface you hover to read. It is one press
    // further away now, and the row at rest offers nothing but its name.
    const { container } = renderRail({ sessions: kids });
    expect(container.textContent).not.toContain('Delete');
    expect(container.querySelectorAll('[data-ui="session-row"]')).toHaveLength(2);
  });

  it('offers the workbench only on a mainline that has branches', () => {
    const { again } = renderRail({ sessions: kids });
    expect(within(openMenu('a mainline')).getByText('Open workbench')).toBeInTheDocument();
    again({ sessions: [root] });
    // Reopened, because the rail re-rendered: only a root with branches has a
    // workbench of its own, so the item is not there for a lone session.
    expect(within(openMenu('a mainline')).queryByText('Open workbench')).toBeNull();
  });

  it('opens the workbench without selecting the session underneath it', () => {
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(within(openMenu('a mainline')).getByText('Open workbench'));
    expect(handlers.onOpenWorkbench).toHaveBeenCalledWith('D:\\firm\\bot');
    expect(handlers.onSelect).not.toHaveBeenCalled();
  });

  it('asks before deleting, and only deletes on the answer', async () => {
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(within(openMenu('a fork')).getByText('Delete'));

    // The question is the layer's `confirm`, not a panel anchored to the row: the row
    // is a menu item now and there is nothing left to anchor a popover to.
    const panel = await screen.findByRole('dialog', { name: 'Delete this session?' });
    expect(panel).toHaveTextContent('a fork');
    expect(handlers.onDelete).not.toHaveBeenCalled();

    fireEvent.click(within(panel).getByRole('button', { name: 'Delete' }));
    // `confirm` resolves a promise, so the call lands a tick later -- which is the
    // whole reason the item is not wired straight to `onDelete`.
    await waitFor(() => expect(handlers.onDelete).toHaveBeenCalledTimes(1));
    expect(handlers.onSelect).not.toHaveBeenCalled();
  });

  it('renames in the row, and sends the new name', () => {
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(within(openMenu('a fork')).getByText('Rename'));

    const field = screen.getByRole('textbox', { name: 'Session name' });
    fireEvent.change(field, { target: { value: 'renamed fork' } });
    fireEvent.keyDown(field, { key: 'Enter' });

    expect(handlers.onRename).toHaveBeenCalledWith('b', 'renamed fork');
  });

  it('leaves an unchanged name alone', () => {
    // Committing the same string would still bump `updated_at` in the core and
    // re-sort the rail under the pointer.
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(within(openMenu('a fork')).getByText('Rename'));
    const field = screen.getByRole('textbox', { name: 'Session name' });
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(handlers.onRename).not.toHaveBeenCalled();
  });

  it('abandons a rename on Escape', () => {
    const { handlers } = renderRail({ sessions: kids });
    fireEvent.click(within(openMenu('a fork')).getByText('Rename'));
    const field = screen.getByRole('textbox', { name: 'Session name' });
    fireEvent.change(field, { target: { value: 'thrown away' } });
    fireEvent.keyDown(field, { key: 'Escape' });
    expect(handlers.onRename).not.toHaveBeenCalled();
    expect(screen.queryByRole('textbox', { name: 'Session name' })).toBeNull();
  });
});
