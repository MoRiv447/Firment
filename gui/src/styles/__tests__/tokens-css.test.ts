/**
 * The bridge between the two token layers.
 *
 * `styles/tokens.ts` is what antd reads and `styles/tokens.css` is what every
 * `.module.css` will read once antd is gone. For the duration of the rewrite
 * both exist, and two sources for the same grey is exactly how
 * `web/src/styles/tokens.css` ended up five values out of date while still
 * claiming to mirror this one.
 *
 * So this test compares them by VALUE, at test time, by importing the real
 * palette rather than re-declaring the numbers. A test that pasted the hexes in
 * here would be a third source and would drift in the same way.
 *
 * Delete this file together with `tokens.ts`, in the stage that removes antd.
 */

import { describe, expect, it } from 'vitest';
import { paletteFor } from '../tokens';

const SOURCES = import.meta.glob('../tokens.css', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

// The glob key is resolved against the project root, not against this file, so
// it is not something to hardcode. Exactly one file matches, and if that ever
// stops being true the test has to say so rather than compare against
// `undefined`.
const MATCHED = Object.entries(SOURCES);
const css = MATCHED[0]?.[1] ?? '';

/** camelCase as written in TS -> the kebab name used in CSS. */
const asVarName = (key: string): string => `--${key.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`)}`;

/**
 * Two ways of writing one colour.
 *
 * `tokens.ts` spells its alphas compact (`rgba(0,0,0,0.32)`) because that is
 * how the inline styles were born; the stylesheet is written with spaces so it
 * stays readable at 3am. Both are correct CSS, so the comparison has to be
 * blind to the difference -- and only to that difference, which is why this
 * normalises whitespace and nothing else.
 */
const canon = (value: string): string =>
  value
    .replace(/\s+/g, ' ')
    .replace(/\(\s+/g, '(')
    .replace(/\s+\)/g, ')')
    .replace(/,\s+/g, ',')
    .trim();

/**
 * `--scroll-thumb` and `--scroll-thumb-hover` are the only colour tokens with no
 * `tokens.ts` twin. The first is a `var(--outline)` reference; the second is the
 * scrollbar hover shade that `index.html` used to invent inline (`#52525b` and
 * `#a1a1aa`, in no token layer at all) and which has no component of its own
 * yet. Anything else added to CSS without a twin is a decision, not a typo, so
 * this list is asserted to be exact rather than consulted as a skip.
 */
const CSS_ONLY = ['--scroll-thumb', '--scroll-thumb-hover'];

const withoutComments = () => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** Read one `{...}` block into a name -> value map. */
function declarations(block: string): Map<string, string> {
  const body = block.match(/\{([^}]*)\}/s);
  expect(body, 'expected a brace-delimited block').not.toBeNull();
  const out = new Map<string, string>();
  for (const match of body![1].matchAll(/([-\w]+)\s*:\s*([^;]+);/g)) {
    out.set(match[1], match[2].trim());
  }
  return out;
}

function schemeBlock(scheme: 'dark' | 'light'): Map<string, string> {
  const source = withoutComments();
  // indexOf, not match: handed a string, `match` compiles a regex, and the
  // brackets of an attribute selector are a character class to a regex.
  const at = source.indexOf(`:root[data-scheme="${scheme}"]`);
  expect(at, `expected a :root[data-scheme="${scheme}"] block`).toBeGreaterThan(-1);
  return declarations(source.slice(at));
}

const dark = schemeBlock('dark');
const light = schemeBlock('light');
const customProps = (m: Map<string, string>) => [...m.keys()].filter((k) => k.startsWith('--'));

describe('tokens.css mirrors tokens.ts', () => {
  it('actually reads the stylesheet', () => {
    // The same self-check the other gate carries: a glob that silently matches
    // nothing would make every assertion below vacuously true.
    expect(MATCHED.map(([path]) => path)).toEqual([expect.stringContaining('tokens.css')]);
    expect(css.length).toBeGreaterThan(1000);
  });

  it('carries one shared key set in both schemes', () => {
    expect(customProps(dark).sort()).toEqual(customProps(light).sort());
    // 37 mirrored + the two scrollbar tokens. A number, on purpose: adding a
    // colour to one scheme and forgetting the other is the bug this whole file
    // exists to catch, and `tokens.ts` catches it with the type system.
    expect(customProps(dark).length).toBe(39);
  });

  it('declares every palette key in both schemes, and nothing extra', () => {
    const expected = Object.keys(paletteFor('dark'))
      .map(asVarName)
      .filter((name) => name !== '--line-strong' && !CSS_ONLY.includes(name));
    for (const name of expected) {
      expect(dark.has(name), `dark scheme is missing ${name}`).toBe(true);
      expect(light.has(name), `light scheme is missing ${name}`).toBe(true);
    }
    const unknown = customProps(dark).filter(
      (name) => !expected.includes(name) && !CSS_ONLY.includes(name),
    );
    expect(unknown, `colour tokens with no tokens.ts twin: ${unknown.join(', ')}`).toEqual([]);
  });

  it('agrees with the dark palette value for value', () => {
    for (const [key, value] of Object.entries(paletteFor('dark'))) {
      const name = key === 'lineStrong' ? '--outline' : asVarName(key);
      expect(canon(dark.get(name) ?? ''), `--${name} (dark)`).toBe(canon(value));
    }
  });

  it('agrees with the light palette value for value', () => {
    for (const [key, value] of Object.entries(paletteFor('light'))) {
      const name = key === 'lineStrong' ? '--outline' : asVarName(key);
      expect(canon(light.get(name) ?? ''), `--${name} (light)`).toBe(canon(value));
    }
  });

  it('knows --surface-raised is not a highlight in the light scheme', () => {
    // The two are the same value there, which makes "paint a raised row on a
    // surface ground" a no-op -- it shipped as an invisible selected session row
    // and looked fine in dark. Raising a surface is a job for the shadow; a
    // highlight is `--hover`, which is a wash in both schemes.
    expect(canon(light.get('--surface-raised') ?? '')).toBe(canon(light.get('--surface') ?? ''));
    expect(canon(dark.get('--surface-raised') ?? '')).not.toBe(canon(dark.get('--surface') ?? ''));
    expect(canon(light.get('--hover') ?? '')).not.toBe(canon(light.get('--surface') ?? ''));
    expect(canon(dark.get('--hover') ?? '')).not.toBe(canon(dark.get('--surface') ?? ''));
  });

  it('merges lineStrong into outline only because the two are equal', () => {
    // The whole basis for having one token instead of two. If a future palette
    // moves either value apart, this fails and the merge has to be undone --
    // rather than quietly changing what one of the two call sites paints.
    expect(paletteFor('dark').outline).toBe(paletteFor('dark').lineStrong);
    expect(paletteFor('light').outline).toBe(paletteFor('light').lineStrong);
  });

  it('sets color-scheme so native controls follow the pinned scheme', () => {
    expect(dark.get('color-scheme')).toBe('dark');
    expect(light.get('color-scheme')).toBe('light');
  });
});

describe('tokens.css number scales', () => {
  /**
   * The scheme-independent block.
   *
   * First declaration wins, because the `prefers-reduced-motion` override lower
   * in the file is also a bare `:root` and a last-wins merge would report every
   * duration as `1ms`. What is being checked here is the ramp the app ships
   * with, not the ramp some users opt into.
   */
  const shared = (() => {
    const merged = new Map<string, string>();
    for (const block of withoutComments().matchAll(/(^|\})\s*:root\s*\{([^}]*)\}/g)) {
      for (const match of block[2].matchAll(/([-\w]+)\s*:\s*([^;]+);/g)) {
        if (!merged.has(match[1])) merged.set(match[1], match[2].trim());
      }
    }
    return merged;
  })();

  it('has a type scale with a named role for every size the old tree used', () => {
    // 137 hardcoded fontSize values, eight distinct sizes. Each one now has to
    // have somewhere to go, or the scale is incomplete and someone will write
    // the literal again.
    const steps = ['--fs-micro', '--fs-meta', '--fs-minor', '--fs-body', '--fs-ui', '--fs-display'];
    for (const step of steps) expect(shared.has(step), `missing ${step}`).toBe(true);
    expect(steps.map((s) => shared.get(s))).toEqual(['10px', '11px', '12px', '13px', '14px', '28px']);
    // The one rung that is not a migration: chrome is scanned and prose is read,
    // so reading text does not share a size with inputs and card titles. 15 is
    // Zed's buffer size, and its UI is 14 -- the same two-role split, from an
    // external ruler rather than from a preference.
    expect(shared.get('--fs-read')).toBe('15px');
  });

  it('has a 4px spacing ramp and the shared row height', () => {
    for (const step of ['--sp-05', '--sp-1', '--sp-2', '--sp-3', '--sp-4', '--sp-5', '--sp-6', '--sp-8']) {
      expect(shared.has(step), `missing ${step}`).toBe(true);
    }
    expect(shared.get('--sp-2')).toBe('8px');
    expect(shared.get('--h-bar')).toBe('44px');
    // One height for every control AND every one-line row: that shared number is
    // what makes a sidebar row, a menu item and an inspector tab look like one
    // system rather than three that happen to sit near each other.
    expect(shared.get('--h-row')).toBe('40px');
    // The small control is a step, not a literal: `data-size="sm"` needs a
    // height and `--h-input` is already the medium one.
    expect(shared.get('--h-min')).toBe('28px');
    expect(shared.get('--h-input')).toBe('36px');
  });

  it('declares the motion the old motion group never got used for', () => {
    expect(shared.get('--t-in')).toBe('160ms');
    expect(shared.get('--t-std')).toBe('200ms');
    expect(shared.get('--t-out')).toBe('120ms');
    expect(shared.get('--ease')).toBe('cubic-bezier(0.2, 0.8, 0.2, 1)');
  });

  it('keeps exactly one z-index ladder', () => {
    // No `--z-menu`: a menu is a floating panel, and a panel layer below the
    // dialog value would paint a Select opened from the settings drawer behind
    // the drawer itself. Ordered, and strictly so -- a tie between two layers is
    // the moment "which one is on top" stops being answerable from the source.
    const layers = ['--z-sticky', '--z-pane', '--z-overlay', '--z-dialog', '--z-float', '--z-toast'];
    const values = layers.map((name) => Number(shared.get(name)));
    expect(values.every((v) => Number.isFinite(v))).toBe(true);
    expect([...values].sort((a, b) => a - b)).toEqual(values);
    expect(values.indexOf(Number(shared.get('--z-float')))).toBeGreaterThan(
      values.indexOf(Number(shared.get('--z-dialog'))),
    );
  });
});
