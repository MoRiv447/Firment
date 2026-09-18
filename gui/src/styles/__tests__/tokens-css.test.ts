/**
 * The structure `tokens.css` has to keep.
 *
 * This file used to be the bridge between two token layers: `styles/tokens.ts`,
 * which antd read, and `styles/tokens.css`, which every `.module.css` reads. Two
 * sources for the same grey is how `web/src/styles/tokens.css` once ended up five
 * values out of date while still claiming to mirror this one, so the bridge
 * compared them by value at test time rather than by eye.
 *
 * The JS copy and the antd layer are both gone, which leaves the half that was
 * never about the copy: one key set in both schemes, a raise step that is real in
 * light, `color-scheme` set for native controls, and the number scales below.
 */

import { describe, expect, it } from 'vitest';

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

/**
 * Values are compared after whitespace is normalised, because the two sides of a
 * comparison are written for different readers: a hex in a scheme block, and a
 * `var()` or an `rgba()` a call site would keep readable at 3am. Both are the same
 * colour, and only that difference is being ignored here.
 */
const canon = (value: string): string =>
  value
    .replace(/\s+/g, ' ')
    .replace(/\(\s+/g, '(')
    .replace(/\s+\)/g, ')')
    .replace(/,\s+/g, ',')
    .trim();

/**
 * There is no longer a list of keys that are CSS-only.
 *
 * It existed to hold the stylesheet to the JS palette's key set: every key had a
 * twin except `--scroll-thumb`/`--scroll-thumb-hover`, and the case asserted that
 * the list was exact. With the copy retired there is no second set to be exact
 * against -- the stylesheet declares what it declares, and what is left to check
 * is that both schemes declare the same thing, which the first case does.
 */
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

describe('tokens.css structure', () => {
  it('actually reads the stylesheet', () => {
    // The same self-check the other gate carries: a glob that silently matches
    // nothing would make every assertion below vacuously true.
    expect(MATCHED.map(([path]) => path)).toEqual([expect.stringContaining('tokens.css')]);
    expect(css.length).toBeGreaterThan(1000);
  });

  it('carries one shared key set in both schemes', () => {
    expect(customProps(dark).sort()).toEqual(customProps(light).sort());
    // Counted rather than derived: adding a colour to one scheme and forgetting the
    // other is the bug this case exists to catch, and a count that has to be edited
    // makes adding one a decision instead of a slip. The texture is not part of the
    // count -- it lives outside both blocks, because it is the same in both schemes
    // and only its strength is declared per scheme.
    expect(customProps(dark).length).toBe(41);
  });

  it('has a light scheme whose raise step is real', () => {
    // This used to assert the opposite -- that `--surface-raised` equalled
    // `--surface` in light, which is what made a selected row invisible there. With
    // the palette on Radix the steps differ, so the trap is gone and the property
    // worth pinning is the one that replaced it.
    expect(canon(light.get('--surface-raised') ?? '')).not.toBe(canon(light.get('--surface') ?? ''));
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

  it('bundles Geist and JetBrains Mono, and never names Inter', () => {
    // Inter was declared for two redesigns and never shipped, so for a while the
    // app rendered the system font while its tokens claimed otherwise. Naming a
    // font is not loading one, and the answer is not a comment: it is a test that
    // fails when someone reaches for the familiar name again.
    expect(shared.get('--ff-sans')).toContain('Geist Variable');
    expect(shared.get('--ff-mono')).toContain('JetBrains Mono Variable');
    expect(shared.get('--ff-sans')).not.toMatch(/Inter/i);
    expect(shared.get('--ff-mono')).not.toMatch(/Inter/i);
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
