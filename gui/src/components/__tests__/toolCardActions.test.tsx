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

  it('has exactly one primary action', () => {
    // The design rule is one CTA per screen; two acid buttons side by side would
    // remove the hierarchy the row exists to express.
    const tiers = quickActionsFor('edit_file').map((a) => a.tier);
    expect(tiers.filter((t) => t === 'primary')).toHaveLength(1);
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
