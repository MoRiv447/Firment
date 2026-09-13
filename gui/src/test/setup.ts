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
 * jsdom ships an empty `document.body`.
 *
 * Two pieces of the UI layer read the document rather than a subtree, so both are
 * dead in a test without this:
 *
 * * `portal.ts` marks `#root` `inert` while a dialog is open -- that is the whole
 *   reason a focus trap does not have to walk the tree, and it cannot be asserted
 *   against a node that does not exist. index.html provides it in the browser.
 * * `useFocusTrap.ts` filters its candidate list by `offsetParent !== null`,
 *   because a control inside a collapsed section must not be a Tab stop. jsdom has
 *   no layout, so it reports `null` for every element, which would leave the trap
 *   with nothing to wrap around.
 *
 * The getter approximates layout the way the tests need it to: a `display: none`
 * subtree has no offset parent, and anything attached to the document does.
 */
if (typeof window !== 'undefined') {
  const root = document.createElement('div');
  root.id = 'root';
  document.body.appendChild(root);

  Object.defineProperty(HTMLElement.prototype, 'offsetParent', {
    configurable: true,
    get(this: HTMLElement) {
      // jsdom's default stylesheet has no `[hidden] { display: none }`, so a
      // collapsed section reads as visible to computed style and only to this
      // check. It belongs in the shim, not in the trap: a browser gets it right.
      if (this.closest('[hidden]')) return null;
      return getComputedStyle(this).display === 'none' ? null : this.parentElement;
    },
  });
}

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
