import { describe, expect, it } from 'vitest';

/**
 * The invariants that make the token layer mean anything.
 *
 * docs/design/tokens.md claims the GUI has one place where a design value is
 * written down. That claim decays one inline literal at a time -- someone needs
 * a slightly rounder corner, types `8`, and now there are two sources of truth
 * and a light scheme that only half-applies. These tests are what stops it: they
 * are cheap, they run with everything else, and they fail at the moment of the
 * mistake rather than at review time.
 *
 * The sources are read through Vite's own `import.meta.glob` rather than
 * `node:fs`, because this project has no `@types/node` (see the
 * `@ts-expect-error` on `process` in vite.config.ts) and a test is not a good
 * reason to add one.
 */

const MODULES = import.meta.glob('../../**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/**
 * Everything under gui/src, minus the token layer and the tests themselves.
 * Tests are excluded because asserting an exact pixel value is their job.
 *
 * Both halves of the exclusion are needed: files beside this one are keyed
 * `./name.ts` rather than `../../styles/__tests__/name.ts`, so the directory is
 * not always in the path.
 */
const FILES = Object.entries(MODULES).filter(
  ([path]) => !/tokens\.ts$/.test(path) && !/(^|\/)__tests__\//.test(path) && !/\.test\.tsx?$/.test(path),
);

/** The offending lines, as `views/ChatView.tsx:12: text`. */
function matches(pattern: RegExp): string[] {
  const hits: string[] = [];
  for (const [path, source] of FILES) {
    const label = path.replace(/^(\.\.\/)+/, '').replace(/^\.\//, '');
    source.split('\n').forEach((line, i) => {
      if (pattern.test(line)) hits.push(`${label}:${i + 1}: ${line.trim()}`);
    });
  }
  return hits;
}

describe('the token layer is the only place a design value is written down', () => {
  it('has no literal corner radius outside the tokens', () => {
    // The radius tiers are 0 / 2 / 4 / 8 and each one means something: a badge
    // is not a card. A bare number is a value nobody chose, and it will not
    // follow the tiers if they ever move.
    expect(matches(/borderRadius:\s*\d/)).toEqual([]);
  });

  it('has no literal colour outside the tokens', () => {
    // This is also what makes the light scheme trustworthy: a literal that
    // happens to read well on #0F0F12 will not be revisited for #F7F7F5.
    expect(matches(/#[0-9a-fA-F]{3,8}\b/)).toEqual([]);
  });

  it('has no literal font stack outside the tokens', () => {
    // `font.sans` / `font.mono` carry the CJK fallback that a hand-written
    // stack forgets, which is what makes Chinese sit wrong next to Latin.
    expect(matches(/fontFamily:\s*['"]/)).toEqual([]);
  });

  it('is actually looking at the source tree', () => {
    // A glob that silently matched nothing would make all of the above pass.
    expect(FILES.length).toBeGreaterThan(15);
    expect(matches(/import \{/).length).toBeGreaterThan(0);
  });
});
