import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { quickActionsFor } from '../../lib/quickActions';
import { ToolCard } from '../ToolCard';
import type { ToolCardState } from '../../types';

/**
 * The action row under an edit, from the mockup's "IN CONTEXT" panel.
 *
 * The assertions that matter are about *when* the row appears and what a click
 * sends: a build button under a tool that did not change anything, or under an
 * edit that is still being written, is an offer to do the wrong thing.
 */

function tool(over: Partial<ToolCardState> = {}): ToolCardState {
  return { seq: 1, name: 'edit_file', args: { path: 'src/main.c' }, status: 'ok', ...over };
}

const DIFF = [
  '--- a/src/main.c',
  '+++ b/src/main.c',
  '@@ -1,2 +1,3 @@',
  ' context',
  '+added one',
  '+added two',
  '-removed one',
  '',
].join('\n');

describe('the phase line and its two-second gate', () => {
  // §16.2: a phase that appears and vanishes under the row above it is worse than silence. The
  // measurement needs a clock, and the clock has to come from the caller: a card that read
  // `Date.now()` during render could only ever be drawn at the instant the test happened to run
  // it, so the gate was unassertable -- and a fixture frozen to a past timestamp showed every
  // restored card as ancient.
  const started = 1_700_000_000_000;
  const running = tool({
    status: 'running',
    progress: 'compiling main.c',
    startedAt: started,
  });
  const phase = () => document.querySelector('[data-ui="tool-progress"]');

  it('stays silent for the first two seconds', () => {
    render(<ToolCard tool={running} now={started + 500} />);
    expect(phase()).toBeNull();
  });

  it('shows the phase once the run is past two seconds', () => {
    render(<ToolCard tool={running} now={started + 2500} />);
    expect(phase()?.textContent).toBe('compiling main.c');
  });

  it('shows nothing at all when no clock was handed over', () => {
    // The renderings that reach this are the ones with no running card to time -- a stored
    // transcript, a permission prompt -- so "no clock" means "no phase", never a guess read off
    // the wall.
    render(<ToolCard tool={running} />);
    expect(phase()).toBeNull();
  });

  it('drops the phase when the call finishes', () => {
    render(
      <ToolCard tool={{ ...running, status: 'ok', endedAt: started + 9000 }} now={started + 9000} />,
    );
    expect(phase()).toBeNull();
  });
});

describe('the card footer', () => {
  it('names the file and the size of the change', () => {
    render(<ToolCard tool={tool({ detail: DIFF })} />);
    expect(screen.getByText('src/main.c · 2 added, 1 removed')).toBeInTheDocument();
  });

  it('says nothing while the call is still running', () => {
    // The band offers to open a diff; opening one that is still being written
    // shows a change that is not the one the card is describing.
    render(<ToolCard tool={tool({ status: 'running', detail: DIFF })} />);
    expect(screen.queryByText(/added, /)).not.toBeInTheDocument();
  });

  it('marks a failed call\'s plain output as the failure', () => {
    const { container } = render(
      <ToolCard tool={tool({ status: 'failed', detail: 'no logic analyzer attached' })} />,
    );
    expect(
      container.querySelector('[data-ui="tool-output"][data-kind="error"]'),
    ).toBeInTheDocument();
  });

  it('does not mark a passed one', () => {
    const { container } = render(
      <ToolCard tool={tool({ status: 'ok', detail: 'linking complete' })} />,
    );
    expect(
      container.querySelector('[data-ui="tool-output"][data-kind="error"]'),
    ).toBeNull();
  });

  it('offers the way out only when somewhere to go was given', () => {
    const onOpenChanges = vi.fn();
    render(<ToolCard tool={tool({ detail: DIFF })} onOpenChanges={onOpenChanges} />);
    fireEvent.click(screen.getByRole('button', { name: /Open in Changes/ }));
    expect(onOpenChanges).toHaveBeenCalledTimes(1);
    // And the band still reads on its own: the handler is the link, not the text.
    expect(screen.getByText('src/main.c · 2 added, 1 removed')).toBeInTheDocument();
  });

  it('renders no link it cannot honour', () => {
    render(<ToolCard tool={tool({ detail: DIFF })} />);
    expect(screen.queryByRole('button', { name: /Open in Changes/ })).not.toBeInTheDocument();
  });

  it('reads the tool\'s own conclusion when it touched no file', () => {
    // A monitor has no path, so the band would otherwise be empty -- and the
    // summary is the one line saying what the call concluded. It used to render
    // only when there was no detail, so a failed capture with a log attached lost
    // its verdict entirely.
    render(
      <ToolCard
        tool={tool({
          name: 'monitor',
          args: { port: '/dev/ttyUSB0' },
          status: 'failed',
          summary: 'no data in 2.0s',
          detail: 'port opened, but no data',
        })}
      />,
    );
    expect(screen.getByText('no data in 2.0s')).toBeInTheDocument();
  });
});

describe('quickActionsFor', () => {
  it('offers build and test after an edit', () => {
    expect(quickActionsFor('edit_file').map((a) => a.label)).toEqual([
      'Build & flash',
      'Run tests',
    ]);
  });

  it('offers nothing after a read', () => {
    // A row that appears under everything stops meaning anything.
    expect(quickActionsFor('read_file')).toEqual([]);
    expect(quickActionsFor('grep')).toEqual([]);
    expect(quickActionsFor('shell')).toEqual([]);
  });

  it('offers its actions as prose, not as a call to action', () => {
    // This asserted "exactly one primary action" -- a CTA and its alternative --
    // which was the right shape for a row of buttons and the wrong one for what this
    // is: the footer of a card that reports something that already happened. A solid
    // button at the bottom of a diff reads as "the page is waiting for you". Both are
    // `quiet` now, so the card has one strong element -- its own edge -- and the
    // actions are two things you could ask next.
    const tiers = quickActionsFor('edit_file').map((a) => a.tier);
    expect(tiers).toEqual(['quiet', 'quiet']);
    expect(tiers).not.toContain('primary');
  });

  it('carries a prompt for every action', () => {
    // An action with no prompt would render a button that sends nothing.
    for (const action of quickActionsFor('edit_file')) {
      expect(action.prompt.trim().length).toBeGreaterThan(0);
    }
  });
});

describe('ToolCard action row', () => {
  it('sends the action it was clicked for', () => {
    const onAction = vi.fn();
    render(<ToolCard tool={tool()} onAction={onAction} />);
    fireEvent.click(screen.getByRole('button', { name: 'Build & flash' }));
    expect(onAction).toHaveBeenCalledWith('Build this change and flash it to the board.');
  });

  it('is not offered while the edit is still running', () => {
    render(<ToolCard tool={tool({ status: 'running' })} onAction={() => {}} />);
    expect(screen.queryByRole('button', { name: 'Build & flash' })).toBeNull();
  });

  it('is absent for a tool that changed nothing', () => {
    render(<ToolCard tool={tool({ name: 'read_file' })} onAction={() => {}} />);
    expect(screen.queryByRole('button', { name: 'Run tests' })).toBeNull();
  });

  it('is absent when the card has no way to send anything', () => {
    // Historical cards are rendered without the chat's send path; a button that
    // cannot act must not be drawn.
    render(<ToolCard tool={tool()} />);
    expect(screen.queryByRole('button', { name: 'Build & flash' })).toBeNull();
  });
});
