import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { EmptyState } from '../EmptyState';
import { LoadingBubble } from '../LoadingBubble';
import { MessageBubble } from '../MessageBubble';
import type { ChatMessage } from '@/lib/types';

/**
 * The three components lifted out of `ChatPage.tsx`.
 *
 * They were bottom-of-file helpers in a 912-line component, which is why none of
 * them had a test: there was nowhere to put one until the web client had a DOM
 * environment. Each assertion below is about something the split could have
 * silently broken.
 */

describe('EmptyState', () => {
  it('offers the workspace by name in the first suggestion', () => {
    render(<EmptyState onSend={() => {}} workspace="thermostat" />);
    // The suggestion is the only place the workspace appears, so a missing
    // interpolation is invisible until someone clicks it.
    expect(screen.getByText('Read the main file in thermostat')).toBeInTheDocument();
  });

  it('sends exactly the text the button shows', () => {
    const onSend = vi.fn();
    render(<EmptyState onSend={onSend} workspace="thermostat" />);
    fireEvent.click(screen.getByRole('button', { name: 'Search for GPIO configuration patterns' }));
    // What is clicked and what is sent have to be the same string, or the
    // suggestion is a lie.
    expect(onSend).toHaveBeenCalledWith('Search for GPIO configuration patterns');
  });

  it('offers every suggestion as a button', () => {
    render(<EmptyState onSend={() => {}} workspace="fw" />);
    expect(screen.getAllByRole('button')).toHaveLength(4);
  });

  it('names the product on an empty chat', () => {
    render(<EmptyState onSend={() => {}} workspace="fw" />);
    expect(screen.getByText('WELCOME TO FIRMENT')).toBeInTheDocument();
  });
});

describe('MessageBubble', () => {
  const assistant: ChatMessage = { role: 'assistant', content: 'Hello there' };
  const user: ChatMessage = { role: 'user', content: 'Hi' };

  it('shows the message text', () => {
    render(<MessageBubble message={assistant} />);
    expect(screen.getByText('Hello there')).toBeInTheDocument();
  });

  it('trims the surrounding whitespace the transcript carries', () => {
    render(<MessageBubble message={{ role: 'assistant', content: '\n\n  padded  \n' }} />);
    expect(screen.getByText('padded')).toBeInTheDocument();
  });

  it('puts the two sides on opposite sides', () => {
    // Which side a message is on is how a transcript is read at a glance.
    const { container: mine } = render(<MessageBubble message={user} />);
    expect(mine.firstElementChild?.className).toContain('flex-row-reverse');

    const { container: theirs } = render(<MessageBubble message={assistant} />);
    expect(theirs.firstElementChild?.className).not.toContain('flex-row-reverse');
  });

  it('names every tool the turn called', () => {
    render(
      <MessageBubble
        message={{
          role: 'assistant',
          content: 'done',
          tool_calls: [
            { id: 'a', name: 'read_file', arguments: {} },
            { id: 'b', name: 'grep', arguments: {} },
          ],
        }}
      />,
    );
    expect(screen.getByText('read_file')).toBeInTheDocument();
    expect(screen.getByText('grep')).toBeInTheDocument();
  });

  it('says it is generating only for the assistant', () => {
    render(<MessageBubble message={assistant} streaming />);
    expect(screen.getByText('generating…')).toBeInTheDocument();

    render(<MessageBubble message={user} streaming />);
    // The user's own message is never "generating".
    expect(screen.getAllByText('generating…')).toHaveLength(1);
  });

  it('survives a message with no content field', () => {
    // The transcript is deserialised from local storage, so a message written
    // by an older build can arrive without one.
    render(<MessageBubble message={{ role: 'assistant' } as ChatMessage} />);
    expect(screen.queryByText('undefined')).toBeNull();
  });
});

describe('LoadingBubble', () => {
  it('says it is thinking', () => {
    render(<LoadingBubble />);
    expect(screen.getByText('THINKING...')).toBeInTheDocument();
  });

  it('stages the three dots rather than blinking them together', () => {
    const { container } = render(<LoadingBubble />);
    const dots = Array.from(container.querySelectorAll('span[style]')) as HTMLElement[];
    const delays = dots.map((d) => d.style.animationDelay);
    expect(delays).toEqual(['0ms', '150ms', '300ms']);
  });
});
