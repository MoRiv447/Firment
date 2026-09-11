/**
 * Vitest environment setup.
 *
 * Before this existed the GUI had vitest but no DOM: the two test files were
 * pure logic (`turnReducer`, `stallHint`) and ran in the node environment. Any
 * component test would have failed on a missing `document`, which is why the
 * test infrastructure had to land before the components, not after.
 *
 * `@testing-library/jest-dom/vitest` is what adds the DOM matchers
 * (`toBeInTheDocument`, `toHaveStyle`, ...) to vitest's `expect`.
 */
import '@testing-library/jest-dom/vitest';

import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

/**
 * Unmount between tests.
 *
 * Testing Library registers this automatically only when the test runner
 * provides global `afterEach`; this project's tests import their helpers
 * explicitly (`import { describe, it } from 'vitest'`) rather than relying on
 * `globals: true`, so the hook has to be wired by hand. Without it the rendered
 * tree from every previous test is still in `document`, and a `getByRole` finds
 * a dozen buttons, or the same button twice.
 */
afterEach(cleanup);

/**
 * jsdom implements no `matchMedia`.
 *
 * antd's responsive components -- `List` among them, via `useBreakpoint` --
 * call it during render, so without this a component test dies on
 * `window.matchMedia is not a function` rather than on the thing under test.
 * `matches: false` is the honest default for a headless run: there is no
 * viewport to report on.
 *
 * The listener methods are no-ops rather than absent, because code that
 * subscribes (theme.ts follows the OS preference) would otherwise throw on the
 * cleanup path.
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
