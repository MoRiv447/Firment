import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import sheet from '../MessageList.module.css?raw';
import { MessageList } from '../MessageList';
import type { ChatMessage } from '../../types';

/**
 * The user's own message is the one bubble drawn on the acid fill.
 *
 * It was rendering at about 1.3:1 in the dark scheme: the bubble read `ink` for
 * its text, and `ink` is near-white on dark, so the message you had just typed
 * was the least legible thing on the screen. Measured rather than eyeballed --
 * 320 near-white pixels inside the bubble in dark, 0 in light.
 *
 * The bug was never "someone picked a bad colour", it is "a fill and its ink
 * were taken from different tokens", and the check for that moved with the
 * values: the pair used to be assembled in JS from `styles/tokens.ts`, so the
 * test pulled the same module and compared computed inline colours. The bubble
 * is styled by a class now, and `--brand-acid` / `--on-acid` re-resolve per
 * scheme in CSS, which leaves one thing worth asserting -- that the two
 * declarations are in the same rule and are the measured pair.
 *
 * jsdom cannot help further here: it does not substitute `var()`, so a
 * computed-style assertion would compare against an empty string in both
 * schemes and pass for the wrong reason.
 */

const RULE = /\.bubble\s*\{([^}]*)\}/;
const declarations = Object.fromEntries(
  [...(RULE.exec(sheet)?.[1] ?? '').matchAll(/([\w-]+)\s*:\s*([^;]+)/g)].map(([, prop, value]) => [
    prop.trim(),
    value.trim(),
  ]),
);

function userMessage(text: string): ChatMessage[] {
  return [{ role: 'user', content: text }];
}

describe('the user bubble pairs its fill with an ink measured against it', () => {
  it('is the transcript row that carries the class', () => {
    render(<MessageList messages={userMessage('flash the board')} />);
    const bubble = screen.getByText('flash the board');
    expect(bubble.getAttribute('data-ui')).toBe('user-bubble');
    // The rule below is only this element's contract if the hashed class still
    // comes from it -- CSS Modules keep the local name in the generated class.
    expect(bubble.className).toMatch(/^_?bubble_/);
  });

  it('takes its ink from the acid pair, not from the text colour', () => {
    expect(declarations.background).toBe('var(--brand-acid)');
    expect(declarations.color).toBe('var(--on-acid)');
    // The specific regression: `ink` is near-white on dark and near-black on
    // light, so it is only accidentally correct in one of the two.
    expect(declarations.color).not.toContain('var(--ink)');
  });

  it('carries no border or shadow of its own', () => {
    // A grey ring around a green block is what made every filled control in the
    // app read as a mistake.
    expect(declarations.border).toBeUndefined();
    expect(declarations['border-width']).toBeUndefined();
    expect(declarations['box-shadow']).toBeUndefined();
  });
});
