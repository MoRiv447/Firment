import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

import { ChatView } from '../ChatView';
import type { SessionDto } from '../../types';

/**
 * The chat pane, which had no test at all until now.
 *
 * That is worth stating plainly, because it is why a missing empty state could sit
 * in the most prominent pane of the application: nothing rendered this component,
 * so nothing could notice. This file is a harness first and a set of assertions
 * second -- it exists so that the next change to this pane has somewhere to land.
 *
 * What it pins is deliberately structural rather than pixel-level: the three
 * states of the transcript (no session / a session with nothing said / a session
 * with messages), and the composer's two rows, because that row moved to the foot
 * of the field in the layout pass and the DOM order is the layout.
 */

function session(over: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 's1',
    cwd: 'D:\\OldStudy66\\Firment',
    provider: 'glm',
    model: 'glm-5.3-flash',
    mode: 'agent',
    thinking: 'off',
    created_at: 0,
    updated_at: 0,
    messages: [],
    ...over,
  };
}

function setup(over: Partial<Parameters<typeof ChatView>[0]> = {}) {
  const onSend = vi.fn();
  const onCancel = vi.fn();
  const view = render(
    <ChatView
      session={session()}
      running={false}
      turn={null}
      infos={[]}
      onSend={onSend}
      onCancel={onCancel}
      {...over}
    />,
  );
  return { onSend, onCancel, view };
}

describe('ChatView: the three states of the transcript', () => {
  it('says so when no session is open', () => {
    setup({ session: null });
    expect(screen.getByText('No session open')).toBeInTheDocument();
  });

  it('says so when a session has said nothing yet', () => {
    // The case the empty state was missing for: a session exists, so the "no
    // session" branch does not run, and there is nothing to render yet.
    setup();
    expect(screen.getByText('Nothing said yet')).toBeInTheDocument();
  });

  it('does not clutter a conversation that has started', () => {
    setup({
      session: session({ messages: [{ role: 'user', content: 'flash the board' }] }),
    });
    expect(screen.queryByText('Nothing said yet')).toBeNull();
    expect(screen.getByText('flash the board')).toBeInTheDocument();
  });

  it('does not claim emptiness while a turn is running', () => {
    // A fresh session's first turn: still no messages, but it is working, not idle.
    setup({ running: true });
    expect(screen.queryByText('Nothing said yet')).toBeNull();
  });
});

describe('ChatView: the composer', () => {
  it('has the field, and the action is not offered when there is nothing to send', () => {
    setup();
    expect(screen.getByRole('textbox', { name: 'Ask the agent' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('offers Stop instead of Send while running', () => {
    setup({ running: true });
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull();
  });

  it('puts the settings after the field, not between you and it', () => {
    // The settings row used to precede the field, which meant the fine print sat
    // between you and what you were about to send. It is below it now, and in a
    // flex column the document order is the visual order -- so this is the
    // assertion that the move happened, rather than one about pixels.
    setup();
    const field = screen.getByRole('textbox', { name: 'Ask the agent' });
    const settings = screen.getByText('glm-5.3-flash');
    expect(
      field.compareDocumentPosition(settings) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByText('D:\\OldStudy66\\Firment')).toBeInTheDocument();
  });

  it('names the mode in the settings row', () => {
    setup({ session: session({ mode: 'plan' }) });
    expect(screen.getByText('plan')).toBeInTheDocument();
  });

  it('shows the notices it was handed', () => {
    setup({ infos: [{ id: 1, text: 'link error: retrying in 3s' }] });
    expect(screen.getByText(/retrying in 3s/)).toBeInTheDocument();
  });
});
