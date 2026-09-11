/**
 * Vitest environment setup for the web client.
 *
 * Two things the node environment never needed and jsdom does not provide on
 * its own. Both are here rather than in a single suite because every component
 * test from now on depends on them.
 */
import '@testing-library/jest-dom/vitest';

import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

/**
 * Unmount between tests.
 *
 * Testing Library registers this automatically only when the runner provides a
 * global `afterEach`, and these suites import their helpers explicitly rather
 * than relying on `globals: true`. Without it the previous tree is still in
 * `document`, and a `getByText` finds a component from a test that already
 * finished.
 */
afterEach(cleanup);

/**
 * jsdom implements no `matchMedia`.
 *
 * Anything that reads a media query during render -- a responsive layout, a
 * component following the OS colour scheme -- throws
 * `window.matchMedia is not a function` without this, which points at the
 * environment rather than at the component under test. `matches: false` is the
 * honest answer headless: there is no viewport to report on.
 */
if (typeof window !== 'undefined' && !window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
