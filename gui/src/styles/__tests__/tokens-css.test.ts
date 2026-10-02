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
 * The two font packages, read from where they are installed.
 *
 * A glob into `node_modules`, because the family a package declares is a fact
 * about the dependency rather than about this file -- and it is the fact a font
 * stack gets wrong. `'Geist Variable'` matched `@fontsource-variable/geist`;
 * `'DM Sans'` does not match `@fontsource-variable/dm-sans`, which declares
 * `'DM Sans Variable'`, and the browser's answer to a name nobody ships is to fall
 * back without saying so.
 */
const MATCHED_FONTS = import.meta.glob(
  [
    '../../../node_modules/@fontsource-variable/dm-sans/wght.css',
    '../../../node_modules/@fontsource/ibm-plex-mono/latin-400.css',
  ],
  { query: '?raw', import: 'default', eager: true },
) as Record<string, string>;

/** The first family in a stack -- the one that has to exist -- with its quotes off. */
const firstFamily = (stack: string) => (stack.split(',')[0] ?? '').trim().replace(/^'|'$/g, '');

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
    //
    // 41 -> 46 on 2026-10-01, when the ramp went olive. `--outline` became
    // `--border-strong` (same role, a name that says which tier it is) and five
    // keys were added: two more text levels (`ink-soft`, `dim` -- the palette only
    // had two, so a path and a heading had to be told apart by size alone), two
    // more edge weights (`border`, `border-active` -- the second carries a
    // selection, which a fill cannot), and `code-bg`, the surface a code block
    // nested inside a card sits on.
    expect(customProps(dark).length).toBe(46);
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
    // The rung that was missing. Before this the scale ran 10-15 and then jumped
    // straight to 28, so a card title had nowhere to go and the tool card shipped
    // its title at `--fs-minor` -- smaller than the body text beside it. 17px is
    // the landing page's card-heading size; its panel heading is 20-21, which at
    // app density is this.
    expect(shared.get('--fs-title')).toBe('17px');
  });

  it('names a CJK face instead of leaving Chinese to system-ui', () => {
    // The complaint this answers was "the font is not right", and the cause was
    // measurable: Latin rendered in a bundled face while Chinese fell through to
    // whatever `system-ui` resolved to, so one line of mixed text had two faces in
    // it. Leaving that implicit is the bug, so the faces are asserted, not the
    // rendering. No CJK face is bundled -- the system has one and shipping it would
    // add megabytes.
    expect(shared.get('--ff-sans')).toContain('Microsoft YaHei UI');
    expect(shared.get('--ff-sans')).toContain('PingFang SC');
    expect(shared.get('--ff-mono')).toContain('Microsoft YaHei UI');
  });

  it('tracks Latin labels out and never tracks CJK the same way', () => {
    // 0.16em is designed for uppercase Latin: narrow, evenly spaced glyphs that
    // open up well. CJK glyphs are already full-width, so the same value reads as
    // a line of separated characters. The landing page cannot settle this -- all
    // of its micro-labels are Latin -- so the two values have to both exist.
    expect(shared.get('--tracking-label')).toBe('0.16em');
    expect(shared.get('--tracking-cjk')).toBe('0.02em');
  });

  it('asks for the family name the imported package actually declares', () => {
    // The trap this exists for has now been walked into twice. The old palette
    // declared Inter for two redesigns and never bundled it, so the app rendered
    // the system font while its tokens claimed otherwise. This restyle declared
    // `'DM Sans'`, which is not what `@fontsource-variable/dm-sans` ships -- the
    // variable packages declare `'<Name> Variable'` -- so the browser matched
    // nothing and fell back, silently, with every test green.
    //
    // Reading the package is the only way to check it: the name is a fact about the
    // dependency, not about this file.
    expect(Object.keys(MATCHED_FONTS)).toHaveLength(2);
    const declared = Object.values(MATCHED_FONTS)
      .flatMap((css) => [...css.matchAll(/font-family:\s*'([^']+)'/g)].map((m) => m[1]));
    expect(new Set(declared)).toEqual(new Set(['DM Sans Variable', 'IBM Plex Mono']));
    // And the tokens have to name exactly those, first in their stacks.
    expect(firstFamily(shared.get('--ff-sans') ?? '')).toBe('DM Sans Variable');
    expect(firstFamily(shared.get('--ff-mono') ?? '')).toBe('IBM Plex Mono');
  });

  it('bundles DM Sans and IBM Plex Mono, and never names Inter', () => {
    // Inter was declared for two redesigns and never shipped, so for a while the
    // app rendered the system font while its tokens claimed otherwise. Naming a
    // font is not loading one, and the answer is not a comment: it is a test that
    // fails when someone reaches for the familiar name again.
    // `Variable` in the name, and that is not decoration: `toContain('DM Sans')`
    // passed for the whole restyle while the stack matched no installed face.
    expect(shared.get('--ff-sans')).toContain('DM Sans Variable');
    expect(shared.get('--ff-mono')).toContain('IBM Plex Mono');
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

  it('declares the one amount a disabled control dims by', () => {
    // A token rather than a literal because the tree had three spellings of it --
    // `Switch` 0.55, `SessionTree` 0.5, and seven controls with no dimming at all.
    // It is load-bearing now rather than cosmetic: `--muted` is olive like every
    // other neutral, so colour cannot carry "disabled" on its own.
    expect(shared.get('--disabled-opacity')).toBe('0.55');
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
