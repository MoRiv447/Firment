import { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import 'antd/dist/reset.css';

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
