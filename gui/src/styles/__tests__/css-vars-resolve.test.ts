import { describe, expect, it } from 'vitest';

/**
 * Every `var(--x)` without a fallback has to resolve.
 *
 * Written because `Pinmap` asked for `--brand-edge` for a week after the token was
 * deleted with the slant. An undeclared custom property makes the declaration
 * invalid at computed-value time and the property falls back to `currentColor` --
 * so it renders, wrongly, in whatever colour the text happens to be, and no gate
 * noticed. Type checks cannot see inside a string, and the palette tests only look
 * at the tokens file itself.
 *
 * The rule is the CSS convention: `var(--x, fallback)` is a deliberate optional
 * override, and is allowed. `var(--x)` is a promise that `--x` exists.
 */

// `../../` from `src/styles/__tests__/` is `src/`, which is every stylesheet in the
// app -- `../` would only reach `src/styles/` and quietly check two files.
const sheets = import.meta.glob('../../**/*.css', { query: '?raw', import: 'default', eager: true }) as Record<
  string,
  string
>;

const declared = new Set<string>();
const referenced = new Map<string, string[]>();

for (const [path, raw] of Object.entries(sheets)) {
  if (path.includes('__tests__')) continue;
  const text = raw.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of text.matchAll(/(--[a-z0-9-]+)\s*:/g)) declared.add(m[1]);
  // only references with no fallback: `var(--x)` and not `var(--x, ...)`
  for (const m of text.matchAll(/var\(\s*(--[a-z0-9-]+)\s*\)/g)) {
    const list = referenced.get(m[1]) ?? [];
    list.push(path);
    referenced.set(m[1], list);
  }
}

describe('no stylesheet asks for a custom property that does not exist', () => {
  it('reads the stylesheets', () => {
    expect(Object.keys(sheets).length).toBeGreaterThan(20);
    expect(declared.size).toBeGreaterThan(50);
  });

  it('resolves every var() that has no fallback', () => {
    const orphans = [...referenced.entries()]
      .filter(([name]) => !declared.has(name))
      .map(([name, where]) => `${name}  <- ${[...new Set(where)].join(', ')}`);
    expect(orphans).toEqual([]);
  });

  it('still catches the case it was written for', () => {
    // `--brand-edge` was deleted with the slant and `Pinmap` kept asking for it.
    expect(declared.has('--brand-edge')).toBe(false);
  });
});
