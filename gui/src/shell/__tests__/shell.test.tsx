import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Bot, Diff } from 'lucide-react';

import { Inspector } from '../Inspector';
import { NotificationBell } from '../NotificationBell';
import { Splitter } from '../Splitter';
import { StatusMenu } from '../StatusBar';
import type { InspectorTab } from '../Inspector';
import type { NotificationEntry } from '../../types';

/**
 * The four things the shell has to get right that a screenshot cannot show.
 *
 * Each is the part that used to be missing rather than the whole component: a
 * drag handle that means the same thing to a keyboard as to a mouse, a tab strip
 * that says which panel is up, a reading that is a control and not a `<span>`
 * wearing a click handler, and a notification row that only looks jumpable when
 * it can actually jump.
 */

const tabs: InspectorTab[] = [
  { key: 'changes', label: 'Changes', icon: Diff, content: 'diffs here' },
  { key: 'agents', label: 'Subagents', icon: Bot, badge: 2, content: 'agents here' },
];

describe('Splitter', () => {
  it('reports its range and its width in the same units it is dragged in', () => {
    render(<Splitter value={320} min={240} max={560} onResize={() => {}} />);
    const handle = screen.getByRole('separator', { name: 'Inspector width' });
    expect(handle).toHaveAttribute('aria-orientation', 'vertical');
    expect(handle).toHaveAttribute('aria-valuenow', '320');
    // `aria-valuenow` alone gets read as a percentage by some screen readers; the
    // text is what makes "320 pixels" the answer to "how wide".
    expect(handle).toHaveAttribute('aria-valuetext', '320 pixels');
  });

  it('widens to the left and clamps at both ends', () => {
    const onResize = vi.fn();
    const { rerender } = render(<Splitter value={552} min={240} max={560} onResize={onResize} />);
    const handle = screen.getByRole('separator');
    // The edge being moved is the column's left one, so `ArrowLeft` is the key
    // that makes it bigger -- the same direction the pointer goes in.
    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(onResize).toHaveBeenLastCalledWith(560);
    rerender(<Splitter value={248} min={240} max={560} onResize={onResize} />);
    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(onResize).toHaveBeenLastCalledWith(240);
  });

  it('leaves an arrow it did not use to the rest of the shell', () => {
    render(<Splitter value={320} min={240} max={560} onResize={() => {}} />);
    const handle = screen.getByRole('separator');
    const event = new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true });
    handle.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });
});

describe('Inspector', () => {
  it('is a tab strip with a panel that names its own tab', () => {
    render(
      <Inspector
        tabs={tabs}
        open
        onToggle={() => {}}
        width={320}
        onResize={() => {}}
      />,
    );
    const strip = screen.getByRole('tablist', { name: 'Inspector' });
    expect(within(strip).getByRole('tab', { name: 'Changes' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    const panel = screen.getByRole('tabpanel');
    expect(panel).toHaveAccessibleName('Changes');
    const selected = within(strip).getByRole('tab', { name: 'Changes' });
    expect(selected).toHaveAttribute('aria-controls', panel.id);
    expect(panel).toHaveAttribute('aria-labelledby', selected.id);
  });

  it('collapses to a rail that reopens the pane you chose, not the first one', () => {
    const onToggle = vi.fn();
    const { rerender } = render(
      <Inspector tabs={tabs} open onToggle={onToggle} width={320} onResize={() => {}} />,
    );
    fireEvent.click(screen.getByRole('tab', { name: /Subagents/ }));
    rerender(
      <Inspector tabs={tabs} open={false} onToggle={onToggle} width={320} onResize={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Subagents, 2' }));
    // The badge belongs to the accessible name because the rail has no room for a
    // second string, and a count you cannot hear is a count that is not there.
    expect(onToggle).toHaveBeenCalled();
    rerender(
      <Inspector tabs={tabs} open onToggle={onToggle} width={320} onResize={() => {}} />,
    );
    expect(screen.getByRole('tab', { name: /Subagents/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });
});

describe('StatusMenu', () => {
  const options = [
    { key: 'agent', label: 'agent (all tools)' },
    { key: 'plan', label: 'plan (read-only tools)' },
  ];

  it('is a button that says it opens a menu, and answers to the key that picks', () => {
    const onSelect = vi.fn();
    render(
      <StatusMenu
        kind="ok"
        label="Mode"
        value="agent"
        options={options}
        onSelect={onSelect}
      />,
    );
    const trigger = screen.getByRole('button', { name: /Mode/ });
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(screen.getByRole('menuitem', { name: 'plan (read-only tools)' }));
    expect(onSelect).toHaveBeenCalledWith('plan');
  });

  it('is disabled rather than opening a menu that does nothing', () => {
    render(
      <StatusMenu kind="ok" label="Mode" value="agent" options={options} onSelect={() => {}} disabled />,
    );
    // Mid-turn the core ignores a mode change, so a menu that opened would be
    // lying about what it can do.
    expect(screen.getByRole('button', { name: /Mode/ })).toBeDisabled();
  });
});

describe('NotificationBell', () => {
  const entry = (over: Partial<NotificationEntry> = {}): NotificationEntry => ({
    id: 'n1',
    ts: Date.now(),
    kind: 'guard',
    title: 'Stalled turn',
    body: 'no events for 90s',
    ...over,
  });

  const bell = (notifications: NotificationEntry[], onOpenSession = vi.fn()) => {
    render(
      <NotificationBell
        notifications={notifications}
        unread={notifications.length}
        onMarkAllRead={() => {}}
        onClear={() => {}}
        onOpenSession={onOpenSession}
      />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: `Notifications, ${notifications.length} unread` }),
    );
    return onOpenSession;
  };

  it('makes a row that can jump a button, and one that cannot just text', () => {
    const onOpenSession = bell([entry({ sid: 's7' }), entry({ id: 'n2' })]);
    // Checked while the panel is up: choosing a row closes it, which is the point.
    expect(
      screen.getAllByText('no events for 90s').filter((node) => node.closest('button') === null),
    ).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: /Stalled turn/ }));
    expect(onOpenSession).toHaveBeenCalledWith('s7');
    expect(screen.queryByRole('region', { name: 'Notifications' })).not.toBeInTheDocument();
  });

  it('says the count twice over, once for the eye and once for the announcement', () => {
    bell([entry(), entry({ id: 'n2' }), entry({ id: 'n3' })]);
    // The badge is `aria-hidden`; the number has to be in the button's name or a
    // screen reader gets "Notifications" and nothing else.
    expect(screen.getByRole('button', { name: 'Notifications, 3 unread' })).toBeInTheDocument();
  });

  it('closes on Escape and hands focus back to the bell', () => {
    bell([entry()]);
    const panel = screen.getByRole('region', { name: 'Notifications' });
    expect(panel).toHaveFocus();
    fireEvent.keyDown(panel, { key: 'Escape' });
    expect(screen.queryByRole('region', { name: 'Notifications' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Notifications, 1 unread' })).toHaveFocus();
  });
});
