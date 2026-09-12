import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MessageList } from '../MessageList';
import { paletteFor, setActivePalette } from '../../styles/tokens';
import type { ChatMessage } from '../../types';

/**
 * The user's own message is the one bubble drawn on the acid fill.
 *
 * It was rendering at about 1.3:1 in the dark scheme: the bubble read
 * `color.ink` for its text, and `ink` is near-white on dark, so the message you
 * had just typed was the least legible thing on the screen. Measured rather
 * than eyeballed -- 320 near-white pixels inside the bubble in dark, 0 in
 * light.
 *
 * The bug is not "someone picked a bad colour", it is "a fill and its ink were
 * taken from different tokens". These assertions pin the pairing in both
 * schemes, because a fix that only holds in one of them is the same bug with a
 * longer fuse.
 */

function userMessage(text: string): ChatMessage[] {
  return [{ role: 'user', content: text }];
}

/** jsdom normalises inline colours to `rgb(r, g, b)`. */
function rgb(hex: string): string {
  const value = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

describe('the user bubble pairs its fill with an ink measured against it', () => {
  for (const mode of ['dark', 'light'] as const) {
    it(`uses onAcid on the acid fill in the ${mode} scheme`, () => {
      setActivePalette(mode);
      const palette = paletteFor(mode);
      render(<MessageList messages={userMessage('flash the board')} />);

      const bubble = screen.getByText('flash the board');
      expect(bubble.style.background).toBe(rgb(palette.brandAcid));
      expect(bubble.style.color).toBe(rgb(palette.onAcid));
      // The specific regression: `ink` is near-white on dark and near-black on
      // light, so it is only accidentally correct in one of the two.
      expect(bubble.style.color).not.toBe(rgb(palette.ink));
    });
  }

  it('carries no border or shadow of its own', () => {
    setActivePalette('dark');
    render(<MessageList messages={userMessage('flash the board')} />);
    // A grey ring around a green block is what made every filled control in the
    // app read as a mistake.
    const bubble = screen.getByText('flash the board');
    expect(bubble.style.border).toBe('');
    expect(bubble.style.borderWidth).toBe('');
    expect(bubble.style.boxShadow).toBe('');
  });
});
