import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

/**
 * The harness itself.
 *
 * This suite asserts nothing about the product -- there is no product component
 * small enough to stand in for it, since `ChatPage.tsx` is the only component
 * the web client has. What it does assert is that the four pieces the DOM
 * environment is made of are actually wired, because each of them fails in a
 * way that looks like a bug in whatever you were really testing:
 *
 *   * jsdom renders at all (a `document` and `window` exist)
 *   * Testing Library can query what it rendered
 *   * the jest-dom matchers are registered
 *   * cleanup runs between tests, so one suite cannot see the last one's tree
 *   * `matchMedia` is shimmed, so a responsive component does not die on it
 *
 * The order matters for the fourth: the test after this one reads the same
 * document and must find nothing.
 */
describe('the DOM test harness', () => {
  it('renders into a document and can query it back', () => {
    render(<p>rendered by the harness probe</p>);
    expect(screen.getByText('rendered by the harness probe')).toBeInTheDocument();
  });

  it('starts the next test with an empty document', () => {
    // If cleanup were not wired, the previous test's paragraph would still be
    // here -- the failure that makes every later assertion in a file lie.
    expect(screen.queryByText('rendered by the harness probe')).toBeNull();
  });

  it('has the jest-dom matchers registered', () => {
    render(<button disabled>probe</button>);
    expect(screen.getByRole('button')).toBeDisabled();
  });

  it('answers a media query instead of throwing', () => {
    // jsdom ships no matchMedia; a responsive component reads it during render.
    expect(typeof window.matchMedia).toBe('function');
    expect(window.matchMedia('(prefers-color-scheme: dark)').matches).toBe(false);
    expect(() => window.matchMedia('(min-width: 600px)')).not.toThrow();
  });
});
