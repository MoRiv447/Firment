import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { SessionSidebar } from '../SessionSidebar';
import type { LocalSession } from '@/lib/localSessions';

/**
 * The session list.
 *
 * Extracted from `ChatPage.tsx`, which is why it has a test now and did not
 * before. Three of the assertions are about behaviour that a refactor could
 * break without anything looking wrong on screen:
 *
 *   * deleting a row must not also select it,
 *   * the selected row has to be distinguishable, and
 *   * the drawer's open state is a transform, not a mount.
 */

function session(over: Partial<LocalSession> = {}): LocalSession {
  return {
    id: 's1',
    title: 'breathing led',
    messages: [],
    createdAt: 0,
    updatedAt: 0,
    ...over,
  } as LocalSession;
}

function setup(over: Partial<Parameters<typeof SessionSidebar>[0]> = {}) {
  const handlers = {
    onClose: vi.fn(),
    onNew: vi.fn(),
    onSelect: vi.fn(),
    onDelete: vi.fn(),
    onOpenSettings: vi.fn(),
  };
  const view = render(
    <SessionSidebar
      open={false}
      sessions={[session()]}
      currentId={null}
      {...handlers}
      {...over}
    />,
  );
  return { ...handlers, view };
}

describe('SessionSidebar', () => {
  it('lists sessions with their titles', () => {
    setup({ sessions: [session({ id: 'a', title: 'breathing led' }), session({ id: 'b', title: 'device setup' })] });
    expect(screen.getByText('breathing led')).toBeInTheDocument();
    expect(screen.getByText('device setup')).toBeInTheDocument();
  });

  it('says so when there is nothing to list', () => {
    setup({ sessions: [] });
    expect(screen.getByText('No sessions yet')).toBeInTheDocument();
  });

  it('selects the row that was clicked', () => {
    const { onSelect } = setup({
      sessions: [session({ id: 'a', title: 'breathing led' }), session({ id: 'b', title: 'device setup' })],
    });
    fireEvent.click(screen.getByText('device setup'));
    expect(onSelect).toHaveBeenCalledWith('b');
  });

  it('deletes without selecting', () => {
    // The delete control sits inside the row's click target. Without
    // stopPropagation the same click would select the session it just deleted,
    // which is how you end up typing into a session that no longer exists.
    const { onSelect, onDelete } = setup({ sessions: [session({ id: 'a' })] });
    fireEvent.click(screen.getByRole('button', { name: /Delete session/ }));
    expect(onDelete).toHaveBeenCalledWith('a');
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('marks which session you are typing into', () => {
    setup({
      sessions: [session({ id: 'a', title: 'first chat' }), session({ id: 'b', title: 'second chat' })],
      currentId: 'b',
    });
    // The selected row is brand-filled; the fill is the only thing that says
    // which one is live.
    const selected = screen.getByText('second chat').closest('div[class*="cursor-pointer"]');
    expect(selected?.className).toContain('surface-brand');
    const other = screen.getByText('first chat').closest('div[class*="cursor-pointer"]');
    expect(other?.className).not.toContain('surface-brand');
  });

  it('opens and closes the drawer with a transform, not a mount', () => {
    const { view } = setup({ open: false });
    const aside = view.container.querySelector('aside');
    // Closed: slid out. The element stays mounted, because a remount would
    // drop the scroll position every time the drawer is dismissed.
    expect(aside?.className).toContain('-translate-x-full');

    view.rerender(
      <SessionSidebar
        open
        onClose={() => {}}
        sessions={[session()]}
        currentId={null}
        onNew={() => {}}
        onSelect={() => {}}
        onDelete={() => {}}
        onOpenSettings={() => {}}
      />,
    );
    expect(view.container.querySelector('aside')?.className).toContain('translate-x-0');
  });

  it('only shows the dismiss backdrop while the drawer is open', () => {
    const { view } = setup({ open: false });
    expect(view.container.querySelector('.fixed.inset-0')).toBeNull();
  });

  it('routes New Session, Settings and the close button', () => {
    const { onNew, onOpenSettings, onClose } = setup({ open: true });
    fireEvent.click(screen.getByRole('button', { name: /New Session/ }));
    expect(onNew).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: /Settings/ }));
    expect(onOpenSettings).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Close menu' }));
    expect(onClose).toHaveBeenCalled();
  });
});
