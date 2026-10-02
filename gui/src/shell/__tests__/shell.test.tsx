import { useState } from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Bot, Diff } from 'lucide-react';

import { Inspector } from '../Inspector';
import { NotificationBell } from '../NotificationBell';
import { Splitter } from '../Splitter';
import { StatusMenu } from '../StatusBar';
import { TitleBar } from '../TitleBar';
import { TitleBarActions } from '../TitleBarActions';
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
  /**
   * The strip with its tab and open state held where the app holds them.
   *
   * The Inspector used to own that state, which is why nothing else in the window
   * could send you to a pane. This is what the app looks like from here.
   */
  function Harness() {
    const [active, setActive] = useState('changes');
    const [open, setOpen] = useState(true);
    return (
      <Inspector
        tabs={tabs}
        active={active}
        onActiveChange={setActive}
        open={open}
        onToggle={() => setOpen((o) => !o)}
        width={320}
        onResize={() => {}}
      />
    );
  }

  it('is a tab strip with a panel that names its own tab', () => {
    render(<Harness />);
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

  it('shows the pane its owner asks for, not the one it was started on', () => {
    // The whole reason the tab is a prop: a tool card's footer says "open the
    // Changes pane", and a strip that keeps its own state cannot be told.
    render(
      <Inspector
        tabs={tabs}
        active="agents"
        onActiveChange={() => {}}
        open
        onToggle={() => {}}
        width={320}
        onResize={() => {}}
      />,
    );
    expect(screen.getByRole('tab', { name: /Subagents/ })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel')).toHaveAccessibleName(/Subagents/);
  });

  it('collapses to a rail that reopens the pane you chose, not the first one', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('tab', { name: /Subagents/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Collapse the inspector' }));
    fireEvent.click(screen.getByRole('button', { name: 'Subagents, 2' }));
    // The badge belongs to the accessible name because the rail has no room for a
    // second string, and a count you cannot hear is a count that is not there.
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

describe('TitleBar', () => {
  it('draws the mark instead of loading one', () => {
    const { container } = render(<TitleBar project="D:/work/firmware" />);
    // The asset this replaced was `/icons/logo-w-64.png` -- the light-ground cut of
    // the logo, sitting nearly invisible on the dark surface that is the default
    // scheme. An `<img>` in the bar is that bug coming back, and no colour test
    // would have caught it, because the file itself is not a token.
    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByRole('banner')).toHaveTextContent(/Firment/);
  });

  it('names the open session, and drops the group when there is none', () => {
    const { rerender } = render(
      <TitleBar project="D:/work/firmware" session="breathing led on PA0" />,
    );
    expect(screen.getByText('breathing led on PA0')).toBeInTheDocument();
    rerender(<TitleBar project="D:/work/firmware" session={null} />);
    expect(screen.queryByText('breathing led on PA0')).not.toBeInTheDocument();
  });
});

describe('the running pill', () => {
  const show = (running?: { tool: string; seconds: number; current?: number; total?: number } | null) =>
    render(
      <TitleBarActions
        mode="dark"
        running={running}
        onToggleTheme={() => {}}
        onOpenSettings={() => {}}
        notifications={[]}
        unread={0}
        onMarkAllRead={() => {}}
        onClear={() => {}}
        onOpenSession={() => {}}
      />,
    );

  it('counts the workflow steps it was given', () => {
    show({ tool: 'build', seconds: 12, current: 2, total: 3 });
    expect(screen.getByText('2 / 3')).toBeInTheDocument();
    expect(screen.getByText('12s')).toBeInTheDocument();
  });

  it('prints no denominator for a turn that never touched the workflow', () => {
    show({ tool: 'thinking', seconds: 4 });
    expect(screen.getByText('thinking')).toBeInTheDocument();
    expect(screen.queryByText(/\/ \d/)).not.toBeInTheDocument();
  });

  it('is absent on the usual case of an idle window', () => {
    show(null);
    expect(screen.queryByText(/s$/)).not.toBeInTheDocument();
  });
});
