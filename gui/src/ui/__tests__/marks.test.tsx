import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Eyebrow } from '../Eyebrow';
import { MARK_PATHS, StatusMark } from '../StatusMark';
import marksCss from '../StatusMark.module.css?raw';
import eyebrowCss from '../Eyebrow.module.css?raw';

/**
 * The two primitives the transcript restyle added.
 *
 * Both are small and both have a rule that is easy to lose: the mark has four shapes
 * that must stay *four*, and the label has a tracking value that is only correct for
 * one of the two scripts this app writes in. Neither failure would show up in a type
 * check, and neither is visible in a diff of a single call site.
 */

describe('StatusMark', () => {
  it('draws a different inside for each state', () => {
    // Four states and four inner paths, one of which is deliberately nothing. The
    // test is over the exported table rather than over rendered svg, because what
    // would break is a state quietly falling back to another one's glyph.
    expect(MARK_PATHS.done).toBeTruthy();
    expect(MARK_PATHS.current).toBeTruthy();
    expect(MARK_PATHS.failed).toBeTruthy();
    expect(MARK_PATHS.pending).toBeNull();
    expect(new Set(Object.values(MARK_PATHS).filter(Boolean)).size).toBe(3);
  });

  it('is decorative unless it is given a label', () => {
    const { container, rerender } = render(<StatusMark state="done" />);
    const mark = container.querySelector('[data-ui="status-mark"]')!;
    expect(mark.getAttribute('aria-hidden')).toBe('true');
    expect(mark.getAttribute('role')).toBeNull();

    // A row already names its tool next to this, so a label there would say the same
    // fact twice; a band has nothing else to say it with.
    rerender(<StatusMark state="done" label="Outcome not recorded" />);
    expect(screen.getByRole('img', { name: 'Outcome not recorded' })).toBeInTheDocument();
  });

  it('carries the state as an attribute, not as a colour from the caller', () => {
    const { container } = render(<StatusMark state="failed" />);
    expect(container.querySelector('[data-ui="status-mark"]')!.getAttribute('data-state')).toBe(
      'failed',
    );
    // The inks live in the stylesheet, keyed on that attribute -- a call site that
    // passed a colour would only work in one scheme.
    expect(marksCss).toContain("[data-state='failed']");
    expect(marksCss).toContain("[data-state='pending']");
  });
});

describe('Eyebrow', () => {
  it('tracks Latin out and leaves CJK alone', () => {
    // 0.16em is designed for uppercase Latin: narrow glyphs that open up well. CJK
    // glyphs are already full-width, so the same number reads as a line of separated
    // characters -- which is why the default is the CJK value and the call site has
    // to opt in.
    expect(eyebrowCss).toContain('letter-spacing: var(--tracking-cjk)');
    expect(eyebrowCss).toContain('.eyebrow[data-latin]');
    expect(eyebrowCss).toContain('letter-spacing: var(--tracking-label)');
  });

  it('uppercases only when asked', () => {
    const { container, rerender } = render(<Eyebrow latin>Sessions</Eyebrow>);
    expect(container.querySelector('[data-ui="eyebrow"]')!.hasAttribute('data-upper')).toBe(true);

    rerender(
      <Eyebrow upper={false} latin={false}>
        本轮进度
      </Eyebrow>,
    );
    const label = container.querySelector('[data-ui="eyebrow"]')!;
    // A Chinese label has no case to change, and asking for one is not harmless: it
    // would make the tracking question harder to read at the call site.
    expect(label.hasAttribute('data-upper')).toBe(false);
    expect(label.hasAttribute('data-latin')).toBe(false);
  });

  it('draws its own rule, so a call site cannot forget it', () => {
    // The 12px brand bar is what makes a 10px tracked line read as a label rather
    // than as leftover text, and it is a pseudo-element so it cannot be placed on
    // the wrong side by the caller.
    expect(eyebrowCss).toContain('::before');
    expect(eyebrowCss).toContain('background: var(--brand-ink)');
  });
});
