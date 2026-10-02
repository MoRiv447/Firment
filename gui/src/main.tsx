import { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
/*
 * The two typefaces, bundled by Vite as woff2 rather than named and hoped for.
 *
 * They are not the same kind of package, and the imports say so:
 *
 * * **DM Sans is variable**, so one file covers every weight the layer asks for
 *   and `wght.css` is the whole story -- the same shape the Geist import had.
 * * **IBM Plex Mono has no variable release**, so it is imported one weight at a
 *   time. Two, because those are the two the layer uses: `--fw-book` (400) for
 *   everything and `--fw-label` (600) for the counts, the review badge and a hunk
 *   header. A weight that is not loaded is not a fallback, it is a *synthesised*
 *   one -- the browser smears the 400 outlines -- which is why this is a short
 *   list rather than `index.css`, which would pull all seven weights and every
 *   subset with them.
 *
 * `latin` only, deliberately. Fontsource's other subsets are cyrillic, greek and
 * vietnamese; bundling them costs their weight in the installer for glyphs this
 * app never draws, and anything outside `latin` -- including every CJK glyph --
 * falls through to the named faces in `--ff-mono` / `--ff-sans` anyway. That
 * fallback is the reason those stacks name a CJK face instead of trusting
 * `system-ui` to pick one.
 */
import '@fontsource-variable/dm-sans/wght.css';
import '@fontsource/ibm-plex-mono/latin-400.css';
import '@fontsource/ibm-plex-mono/latin-600.css';
/**
 * `?showcase=1` opens the primitive gallery instead of the app.
 *
 * Dev-only and behind a dynamic import: `import.meta.env.DEV` is folded to `false`
 * by the production build, so the branch -- and with it the chunk -- disappears from
 * what ships. The gallery exists because a primitive that has never been drawn is
 * not finished: a chip that pairs two tokens measured against different grounds, or
 * a panel that lands off-screen, still type-checks.
 */
const Showcase = lazy(() => import('./dev/Showcase').then((m) => ({ default: m.Showcase })));

const gallery = import.meta.env.DEV && new URLSearchParams(window.location.search).has('showcase');

ReactDOM.createRoot(document.getElementById('root')!).render(
  gallery ? (
    <Suspense fallback={null}>
      <Showcase />
    </Suspense>
  ) : (
    <App />
  ),
);
