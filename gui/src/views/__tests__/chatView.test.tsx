import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import { ChatView } from '../ChatView';
import type { RunningTurn, SessionDto } from '../../types';

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
 * with messages), and the composer's frame: the text, the two setting chips and the
 * action are one box, and the DOM order inside it is the layout.
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
  it('has the field, and the action is offered even when the field is empty', () => {
    // It is not disabled on an empty field, and that is a measurement rather than a
    // preference: the design's send button is the solid acid, and in light a fill
    // pale enough to keep a dark ink readable stops looking green at all. So the
    // field's emptiness is not a state of this button -- `send()` returns on an empty
    // input either way, and the disabled state is kept for the places it means
    // something real (`SerialView`'s Start, while the port is not open).
    setup();
    expect(screen.getByRole('textbox', { name: 'Ask the agent' })).toBeInTheDocument();
    const send = screen.getByRole('button', { name: 'Send' });
    expect(send).toBeEnabled();
    expect(send).toHaveAttribute('data-tier', 'primary');
  });

  it('sends nothing when the field is empty', () => {
    const { onSend } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(onSend).not.toHaveBeenCalled();
  });

  it('offers Stop instead of Send while running', () => {
    setup({ running: true });
    // Both actions are a glyph in a square now, so the name exists only as an
    // accessible one -- which is the thing a future "just drop the label" change
    // would break, and nothing else here would notice.
    const stop = screen.getByRole('button', { name: /Stop/ });
    expect(stop).toBeInTheDocument();
    expect(stop).toHaveTextContent('');
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull();
  });

  it('names the square send button without a word on it', () => {
    setup();
    const send = screen.getByRole('button', { name: 'Send' });
    expect(send).toHaveTextContent('');
  });

  it('measures a running tool against the clock it was handed', () => {
    // The row beside the spinner used to call `Date.now()` during render, so a
    // fixture pinned to a past timestamp printed a two-year-old build, and no test
    // could freeze a running turn to ask what it says at nine seconds.
    const started = 1_700_000_000_000;
    setup({
      running: true,
      now: started + 9_000,
      turn: {
        text: '',
        tools: { 1: { seq: 1, name: 'build', args: {}, status: 'running', startedAt: started } },
        thinking: '',
        startedAt: started,
        finished: false,
      },
    });
    const row = screen.getByText(/^tool /);
    expect(row.textContent).toMatch(/^tool 9/);
  });

  it('is one box: the text, the settings and the action share a frame', () => {
    // The composer used to be a frameless field with a row of readings under both
    // it and the button -- three things stacked where the design has one object.
    // The structure is the assertion: the nearest ancestor the two chips and the
    // textarea have in common is the element the Send button is in as well.
    setup();
    const field = screen.getByRole('textbox', { name: 'Ask the agent' });
    const mode = screen.getByRole('button', { name: 'agent' });
    const send = screen.getByRole('button', { name: 'Send' });
    // chips > chip, foot > chips, frame > foot.
    const frame = mode.parentElement?.parentElement?.parentElement;
    expect(frame).toBeTruthy();
    expect(frame!.contains(field)).toBe(true);
    expect(frame!.contains(send)).toBe(true);
  });

  it('puts the settings after the text, so nothing sits between you and it', () => {
    setup();
    const field = screen.getByRole('textbox', { name: 'Ask the agent' });
    const mode = screen.getByRole('button', { name: 'agent' });
    expect(
      field.compareDocumentPosition(mode) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('shows the mode as two chips, with the session’s own one lit', () => {
    // Two chips and not a menu: the mode has two answers and both fit, and a control
    // whose options are visible is one you do not have to open to understand.
    setup({ session: session({ mode: 'plan' }) });
    expect(screen.getByRole('button', { name: 'agent' })).not.toHaveAttribute('data-on');
    expect(screen.getByRole('button', { name: 'plan' })).toHaveAttribute('data-on');
  });

  it('switches the mode when a chip is pressed', () => {
    const onMode = vi.fn();
    setup({ onMode });
    fireEvent.click(screen.getByRole('button', { name: 'plan' }));
    expect(onMode).toHaveBeenCalledWith('plan');
  });

  it('names the thinking level on its chip', () => {
    setup({ session: session({ thinking: 'max' }) });
    expect(screen.getByRole('button', { name: /thinking · max/ })).toBeInTheDocument();
  });

  it('shows the notices it was handed', () => {
    setup({ infos: [{ id: 1, text: 'link error: retrying in 3s' }] });
    expect(screen.getByText(/retrying in 3s/)).toBeInTheDocument();
  });
});

describe('ChatView: the counter above the transcript', () => {
  it('counts the plan when the session has one', () => {
    setup({ progress: { done: 2, total: 5 } });
    // Zero-padded so the row does not change width as the numerator moves, and in
    // `done / total` rather than a call count: a turn that retried a failing build
    // twice moves the call count and not the plan.
    expect(screen.getByText('02 / 05')).toBeInTheDocument();
    expect(screen.queryByText(/tool calls/)).not.toBeInTheDocument();
  });

  it('falls back to the call count when there is no plan', () => {
    setup({ progress: null });
    expect(screen.getByText('0 tool calls')).toBeInTheDocument();
  });
});

describe('ChatView: what a finished edit offers next', () => {
  const started = 1_700_000_000_000;
  // A card the turn has already closed: the row's own precondition is met, which is the
  // point — the thing that has to stop the offer is the TURN, and a card cannot see it.
  const turn: RunningTurn = {
    text: '',
    thinking: '',
    startedAt: started,
    finished: false,
    tools: {
      3: { seq: 3, name: 'edit_file', args: {}, status: 'ok', summary: 'two hunks applied' },
    },
  };

  const openEditCard = () => {
    fireEvent.click(screen.getAllByRole('button', { name: /edit_file/ })[0]);
    return screen.queryByRole('button', { name: 'Build & flash' });
  };

  it('offers the actions for an edit that finished, once the turn is over', () => {
    setup({ turn });
    expect(openEditCard()).toBeInTheDocument();
  });

  it('offers nothing while the turn is still running', () => {
    // Both actions send a NEW request through the composer's path, and `start_turn` refuses
    // while the session's slot is held — so mid-turn the only thing pressing one can do is
    // produce "Not sent". An offer that cannot be taken is not an offer; the row disappears,
    // the same way it does for a card with nowhere to send to.
    setup({ turn, running: true });
    expect(openEditCard()).toBeNull();
    expect(screen.queryByRole('button', { name: 'Run tests' })).toBeNull();
  });
});

describe('ChatView: the composer when a send is refused', () => {
  it('puts the text back in the field when the kernel refuses it', async () => {
    // `send()` clears the field before the call lands, because a composer that keeps the text
    // through the round trip shows the same message twice — the transcript has it optimistically.
    // A refusal then removes it from the transcript too, and without this the text existed
    // nowhere on screen: the click did nothing AND ate what was typed.
    const refuse = vi.fn().mockRejectedValue(new Error('this session already has a turn running'));
    setup({ onSend: refuse });
    const field = screen.getByRole('textbox', { name: 'Ask the agent' });
    fireEvent.change(field, { target: { value: 'flash the board' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(field).toHaveValue('');
    await waitFor(() => expect(field).toHaveValue('flash the board'));
    expect(refuse).toHaveBeenCalledWith('flash the board');
  });

  it('leaves the field empty when the send is accepted', async () => {
    const accept = vi.fn().mockResolvedValue(undefined);
    setup({ onSend: accept });
    const field = screen.getByRole('textbox', { name: 'Ask the agent' });
    fireEvent.change(field, { target: { value: 'flash the board' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(accept).toHaveBeenCalledTimes(1));
    expect(field).toHaveValue('');
  });
});
