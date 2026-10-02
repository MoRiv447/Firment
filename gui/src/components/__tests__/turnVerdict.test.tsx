import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import { TurnVerdict } from '../TurnVerdict';
import type { ToolCardState } from '../../types';

/**
 * The band that ends a turn.
 *
 * What is being pinned is the part a summary gets wrong silently: it can name a
 * step that never ran, or grade a whole turn as failed because one clause of it
 * was. Both read fine until someone compares the band with the cards above it.
 */

const T0 = 1_000;

function tool(over: Partial<ToolCardState> & { name: string }): ToolCardState {
  return {
    seq: 1,
    args: {},
    status: 'ok',
    startedAt: T0,
    endedAt: T0 + 400,
    ...over,
  };
}

describe('TurnVerdict', () => {
  it('names the steps that ran, in their own words', () => {
    render(
      <TurnVerdict
        tools={[
          tool({ name: 'build', seq: 1 }),
          tool({ name: 'flash', seq: 2 }),
        ]}
        now={T0 + 900}
        turnStartedAt={T0}
      />,
    );
    expect(screen.getByText('Build passed · Flash passed')).toBeInTheDocument();
    expect(screen.getByText(/turn /)).toBeInTheDocument();
  });

  it('says nothing about a turn that never touched the workflow', () => {
    // A chat that only read files has no build to grade, and a band that listed
    // three steps that did not happen reads as a failure log.
    const { container } = render(
      <TurnVerdict tools={[tool({ name: 'read_file' })]} now={T0 + 900} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('keeps the shape when one step failed, and marks only the edge', () => {
    render(
      <TurnVerdict
        tools={[
          tool({ name: 'build', seq: 1, status: 'failed' }),
          tool({ name: 'flash', seq: 2 }),
        ]}
        now={T0 + 900}
      />,
    );
    const band = screen.getByText('Build failed · Flash passed').closest('[data-ui="turn-verdict"]');
    expect(band).toHaveAttribute('data-tone', 'failed');
  });

  it('counts up while the turn is still going', () => {
    render(
      <TurnVerdict
        tools={[tool({ name: 'build', seq: 1, status: 'running', endedAt: undefined })]}
        now={T0 + 5_000}
        turnStartedAt={T0}
      />,
    );
    expect(screen.getByText('Build running')).toBeInTheDocument();
  });
});
