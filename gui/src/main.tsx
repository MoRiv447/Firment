import { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
/* The two typefaces, bundled by Vite as woff2 rather than named and hoped for.
   Both are variable, so one file each covers every weight the layer asks for.
   `wght.css` references every subset fontsource ships and gives each one its own
   `unicode-range`, which means the browser fetches latin and nothing else -- and
   Chinese falls through to the system font, which is the only affordable answer:
   a bundled CJK face is megabytes. */
import '@fontsource-variable/geist/wght.css';
import '@fontsource-variable/jetbrains-mono/wght.css';
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
