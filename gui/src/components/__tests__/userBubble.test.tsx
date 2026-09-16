import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import sheet from '../MessageList.module.css?raw';
import { MessageList } from '../MessageList';
import type { ChatMessage } from '../../types';

/**
 * The user's own message.
 *
 * This file used to pin a fill/ink pair, and the reason it existed is worth
 * keeping even now that the pair is gone: the bubble once rendered at about 1.3:1
 * in the dark scheme, because its text took `ink` (near-white on dark) while its
 * ground was acid. Measured rather than eyeballed -- 320 near-white pixels inside
 * the bubble in dark, 0 in light. The bug was never "someone picked a bad colour",
 * it was "a fill and its ink were taken from different tokens", and the check for
 * that moved wherever the fill went.
 *
 * The fill went. Neither of the two references puts a filled, saturated block
 * behind what the user typed, and the acid was being spent twice -- bubble and
 * composer -- so neither of the two primary actions on screen read as primary.
 * What the old test protected has not been dropped, it has been restated: a rule
 * with no fill has no pair to keep in step, and that is the assertion here.
 *
 * jsdom cannot help further: it does not substitute `var()`, so a computed-style
 * assertion would compare against an empty string in both schemes and pass for the
 * wrong reason.
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

describe('the user message is text, not a filled control', () => {
  it('is the transcript row that carries the class', () => {
    render(<MessageList messages={userMessage('flash the board')} />);
    const bubble = screen.getByText('flash the board');
    expect(bubble.getAttribute('data-ui')).toBe('user-bubble');
    // The rule below is only this element's contract if the hashed class still
    // comes from it -- CSS Modules keep the local name in the generated class.
    expect(bubble.className).toMatch(/^_?bubble_/);
  });

  it('has no fill, so there is no fill/ink pair left to keep in step', () => {
    expect(declarations.background).toBeUndefined();
    expect(declarations.color).toBeUndefined();
  });

  it('is told apart by weight, which needs no ground of its own', () => {
    expect(declarations['font-weight']).toBe('var(--fw-label)');
    // The same measure as the assistant side: two speakers, one column.
    expect(declarations['max-width']).toBe('88%');
  });

  it('is not a chip: no border, no radius, no shadow', () => {
    // A grey ring around a filled block is what made every filled control in the
    // old app read as a mistake, and a radius without a fill marks nothing.
    expect(declarations.border).toBeUndefined();
    expect(declarations['border-width']).toBeUndefined();
    expect(declarations['border-radius']).toBeUndefined();
    expect(declarations['box-shadow']).toBeUndefined();
  });
});
