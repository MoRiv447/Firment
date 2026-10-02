import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import sheet from '../MessageList.module.css?raw';
import { MessageList } from '../MessageList';
import type { ChatMessage } from '../../types';

/**
 * The user's own message.
 *
 * Three shapes have been tried here and this file has pinned two of them, so the
 * history is the specification.
 *
 *  1. The acid fill. It rendered at about 1.3:1 in the dark scheme -- 320 near-white
 *     pixels inside the bubble in dark and 0 in light, measured rather than guessed
 *     -- because its text took `ink` while its ground took `--brand-acid`. The lesson
 *     was never "a bad colour": it was that a fill and its ink must come from one
 *     measured pair.
 *  2. No fill at all. Weight alone was to say who was speaking, and the acid stopped
 *     being spent twice. Both true, and it failed for a different reason: a paragraph
 *     in a column of paragraphs is the assistant's, so the reader was left to infer
 *     the other one from a difference in weight with nothing to compare it against.
 *  3. A block again -- right-aligned, on `--surface-raised`, with a `--border` edge.
 *     That ground and `--ink` are a pair the palette gate already asserts (16.10:1
 *     dark, 14.68:1 light), and it spends no brand colour, so the composer's Send is
 *     still the only acid on screen. Right alignment is the signal that needs neither
 *     colour nor a legend.
 *
 * jsdom cannot check the rendering: it does not substitute `var()`, so a
 * computed-style assertion would compare against an empty string in both schemes and
 * pass for the wrong reason. The rule is read as text instead.
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

describe('the user message is a block, and it is right-aligned', () => {
  it('is the transcript row that carries the class', () => {
    render(<MessageList messages={userMessage('flash the board')} />);
    const bubble = screen.getByText('flash the board');
    expect(bubble.getAttribute('data-ui')).toBe('user-bubble');
    // The rule below is only this element's contract if the hashed class still
    // comes from it -- CSS Modules keep the local name in the generated class.
    expect(bubble.className).toMatch(/^_?bubble_/);
  });

  it('takes a ground and an edge, from the pair the palette gate asserts', () => {
    // `--surface-raised` and `--ink` are asserted together in
    // `styles/__tests__/tokens.test.ts`. This is the assertion that the bubble is a
    // *measured* pair, and not the two tokens the 1.3:1 bug happened to pick.
    expect(declarations.background).toBe('var(--surface-raised)');
    expect(declarations.border).toBe('1px solid var(--border)');
    expect(declarations.color).toBeUndefined();
  });

  it('spends no brand colour, so the composer still owns the acid', () => {
    // The second attempt was right about this and it still holds: one saturated
    // action per screen.
    expect(JSON.stringify(declarations)).not.toContain('--brand');
    expect(JSON.stringify(declarations)).not.toContain('--acid');
  });

  it('is told apart by where it sits', () => {
    expect(declarations['align-self']).toBe('flex-end');
    expect(declarations['margin-inline-start']).toBe('auto');
    // Narrower than the assistant's 88%: a bubble has edges, so its ragged edge is
    // visible where a paragraph's is the column's.
    expect(declarations['max-width']).toBe('68%');
  });

  it('keeps the line breaks someone typed, and carries no shadow', () => {
    expect(declarations['white-space']).toBe('pre-wrap');
    // Elevation is the edge plus the surface step; the dark scheme has no shadow
    // ladder at all, so a bubble with one would be a bubble that only works in light.
    expect(declarations['box-shadow']).toBeUndefined();
  });
});
