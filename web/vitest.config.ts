import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

/**
 * Vitest, for the web client.
 *
 * The project had vitest and no config at all, which meant the node
 * environment: fine for the seven pure-logic suites under `src/lib`, and
 * useless for anything that renders. `ChatPage.tsx` is 912 lines of component
 * and cannot be taken apart without a way to assert what it renders, so the DOM
 * environment is the prerequisite, not a follow-up.
 *
 * `jsx` comes from tsconfig (`react-jsx`), so esbuild transforms TSX without a
 * React plugin here. The `@/` alias has to be repeated because Next resolves it
 * from tsconfig `paths`, which vitest does not read.
 */
export default defineConfig({
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
