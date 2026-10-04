import { describe, expect, it } from 'vitest';
import { body, named, scheme } from './readPalette';

/** The palette as it is shipped: read out of `tokens.css`, not a copy of it. */
const dark = named('dark');
const light = named('light');

/**
 * Short aliases for the table below.
 *
 * The cases list went from 19 rows to 51 when the palette gained two text levels
 * and three edge weights, and at that length `light.` and `dark.` on every row are
 * most of what you scroll past. The rows name the palette so they cannot read one
 * scheme's colour while claiming to be the other; these are only shorter names for
 * the same two objects.
 */
const D = dark;
const Lt = light;

/**
 * The palette, held to the two rules that cannot be reviewed by eye.
 *
 * Contrast is arithmetic, so it is checked here rather than trusted. The `cases` table
 * below measures the shipped palette against the floors it has to clear; the block at the
 * end of this file reads `docs/design/tokens.md` and recomputes every ratio that document
 * tabulates, which is the half that used to be a transcription and therefore could not be
 * wrong about anything, including the time it was wrong.
 */

/** WCAG 2.1 relative luminance. */
function luminance(hex: string): number {
  const value = hex.replace('#', '');
  const channels = [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16) / 255);
  const [r, g, b] = channels.map((c) =>
    c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4),
  );
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(fg: string, bg: string): number {
  const a = luminance(fg);
  const b = luminance(bg);
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

/** AA for body text. */
const AA = 4.5;

/**
 * Whether a measurement rounds to the number a claim quotes.
 *
 * Both the document and the stylesheet's comments quote two decimals, so that is the
 * precision a claim is made at and the precision it has to hold. Subtracting half a step of
 * the last quoted digit — the bound this file used to use, inherited from `toBeCloseTo(_, 1)` —
 * is exactly the wrong shape near white, where the differences worth arguing about are 0.05
 * apart: `1.07:1` and `1.12:1` are two different grounds and two different decisions.
 * Rounding is compared rather than subtracted, so 4.7350 still counts as the quoted 4.74.
 */
const agrees = (actual: number, quoted: number) => actual.toFixed(2) === quoted.toFixed(2);

describe('the palette cannot drift between schemes', () => {
  it('defines the same key set in dark and light', () => {
    expect(Object.keys(light).sort()).toEqual(Object.keys(dark).sort());
  });
});

describe('text is readable on every ground it is used on', () => {
  it('gives the light scheme a focus ring a keyboard user can see', () => {
    // A focus ring is a shape, so 3:1 is the floor, not 4.5. The light ring cannot
    // be the acid: `#4d7c0f` is a fill, and the same green as a 2px ring on white
    // is too weak. The dark ring is the lime, which clears the floor fifteen times
    // over. They are not required to equal `brandInk`: the measurement is the part
    // that matters, and the two schemes reach it two different ways.
    expect(contrast(light.focusRing, '#FFFFFF')).toBeGreaterThanOrEqual(3);
    expect(contrast(dark.focusRing, dark.bg)).toBeGreaterThan(3);
  });

  // [label, foreground, background, the ratio, which is a measurement of the
  // palette rather than a quote from a document -- rows name the palette so they
  // cannot read one scheme's colour while claiming to be the other].
  const cases: Array<[string, string, string, number]> = [
    // Grounds and edges -- the four text levels, each on every surface it can
    // land on. `dim` is the one that binds: it is the quietest rung and it still
    // has to clear AA on a card, on the code block inside it, and on a selection.
    ['light ink on bg', Lt.ink, Lt.bg, 16.43],
    ['light ink on surface', Lt.ink, Lt.surface, 15.68],
    ['light ink on raised', Lt.ink, Lt.surfaceRaised, 14.68],
    ['light inkSoft on surface', Lt.inkSoft, Lt.surface, 9.71],
    ['light inkSoft on raised', Lt.inkSoft, Lt.surfaceRaised, 9.10],
    ['light muted on bg', Lt.muted, Lt.bg, 6.06],
    ['light muted on surface', Lt.muted, Lt.surface, 5.78],
    ['light muted on raised', Lt.muted, Lt.surfaceRaised, 5.41],
    ['light dim on surface', Lt.dim, Lt.surface, 5.06],
    ['light dim on raised', Lt.dim, Lt.surfaceRaised, 4.74],
    ['light dim on codeBg', Lt.dim, Lt.codeBg, 4.60],
    ['dark ink on bg', D.ink, D.bg, 17.29],
    ['dark ink on surface', D.ink, D.surface, 16.49],
    ['dark inkSoft on raised', D.inkSoft, D.surfaceRaised, 12.73],
    ['dark muted on bg', D.muted, D.bg, 8.84],
    ['dark muted on surface', D.muted, D.surface, 8.43],
    ['dark dim on surface', D.dim, D.surface, 5.48],
    ['dark dim on raised', D.dim, D.surfaceRaised, 5.35],
    ['dark dim on codeBg', D.dim, D.codeBg, 5.67],
    // Brand, and where the two values that make it up are not interchangeable.
    ['light brandInk on surface', Lt.brandInk, Lt.surface, 5.48],
    ['light brandInk on raised', Lt.brandInk, Lt.surfaceRaised, 5.13],
    ['dark brandInk on surface', D.brandInk, D.surface, 14.82],
    ['onAcid on brandAcid light', Lt.onAcid, Lt.brandAcid, 4.99],
    ['onAcid on brandAcid dark', D.onAcid, D.brandAcid, 13.28],
    // The spent fill, as its own pair. This is what an empty composer shows, which is
    // most of the time -- and the light value used to be a `color-mix` at 24%, which
    // came to 4.40:1. Nobody had measured it, because a rule that mixes two colours
    // is not a pair the table can name.
    ['onAcidDim on brandAcidDim light', Lt.onAcidDim, Lt.brandAcidDim, 4.80],
    ['onAcidDim on brandAcidDim dark', D.onAcidDim, D.brandAcidDim, 6.66],
    // Status and the two diff families -- the step states share their values
    // with the diff one, so these also pin that they stay shared.
    ['light successInk on successBg', Lt.successInk, Lt.successBg, 5.43],
    ['dark successInk on successBg', D.successInk, D.successBg, 8.72],
    ['light infoInk on infoBg', Lt.infoInk, Lt.infoBg, 8.69],
    ['dark infoInk on infoBg', D.infoInk, D.infoBg, 10.52],
    ['light warnInk on warnBg', Lt.warnInk, Lt.warnBg, 7.26],
    ['dark warnInk on warnBg', D.warnInk, D.warnBg, 11.88],
    ['light diffAddedInk on diffAddedBg', Lt.diffAddedInk, Lt.diffAddedBg, 9.96],
    ['dark diffAddedInk on diffAddedBg', D.diffAddedInk, D.diffAddedBg, 12.14],
    ['light diffRemovedInk on diffRemovedBg', Lt.diffRemovedInk, Lt.diffRemovedBg, 9.28],
    ['dark diffRemovedInk on diffRemovedBg', D.diffRemovedInk, D.diffRemovedBg, 10.89],
    ['light stepDoneInk on stepDoneBg', Lt.stepDoneInk, Lt.stepDoneBg, 9.96],
    ['dark stepDoneInk on stepDoneBg', D.stepDoneInk, D.stepDoneBg, 12.14],
    ['light stepFailedInk on stepFailedBg', Lt.stepFailedInk, Lt.stepFailedBg, 9.28],
    ['dark stepFailedInk on stepFailedBg', D.stepFailedInk, D.stepFailedBg, 10.89],
    ['light stepPendingInk on bg', Lt.stepPendingInk, Lt.bg, 16.43],
    ['dark stepPendingInk on bg', D.stepPendingInk, D.bg, 5.75],
    // Everything that can be painted ON a selection. `--selection` is not only a
    // selected row: the running tool card, the verdict band and a pressed icon
    // button all use it, so the list is longer than "what a row contains".
    ['light ink on selection', Lt.ink, Lt.selection, 14.01],
    ['dark ink on selection', D.ink, D.selection, 13.82],
    ['light muted on selection', Lt.muted, Lt.selection, 5.17],
    ['dark muted on selection', D.muted, D.selection, 7.06],
    ['light dim on selection', Lt.dim, Lt.selection, 4.53],
    ['dark dim on selection', D.dim, D.selection, 4.59],
    ['light brandInk on selection', Lt.brandInk, Lt.selection, 4.90],
    ['dark brandInk on selection', D.brandInk, D.selection, 12.42],
    ['light successInk on selection', Lt.successInk, Lt.selection, 5.37],
    ['dark successInk on selection', D.successInk, D.selection, 8.48],
    // `onSelection` exists so a call site never reaches for `ink` and gets the
    // wrong one when a scheme makes them differ. They happen to agree now.
    ['selection ink on selection light', Lt.onSelection, Lt.selection, 14.01],
    ['selection ink on selection dark', D.onSelection, D.selection, 13.82],
  ];

  for (const [label, fg, bg, quoted] of cases) {
    it(`${label} is ${quoted}:1`, () => {
      const actual = contrast(fg, bg);
      // Two decimals: the document quotes rounded values, and a tighter bound
      // would fail on the rounding rather than on the colour.
      expect(actual).toBeCloseTo(quoted, 1);
      expect(actual).toBeGreaterThanOrEqual(AA);
    });
  }


  it('keeps the hover wash from sinking the muted label', () => {
    // The wash is only a wash if a label on top of it still passes. (Dark is
    // skipped: its wash is a translucent overlay, and the luminance helper reads
    // hexes.) What the wash IS changed with the restyle -- a warm neutral became a
    // green one -- so this is the case that says it is still a wash and not a
    // second surface.
    const p = light;
    expect(contrast(p.muted, p.hover)).toBeGreaterThanOrEqual(AA); // 5.14
    expect(contrast(p.ink, p.hover)).toBeGreaterThanOrEqual(AA); // 13.93
  });


  it('keeps a 2px brand mark visible on the ground it is drawn on', () => {
    // stepRule is a shape, not text -- the current-step underline, the live
    // inspector tab, the todo progress bar. Acid on a light ground is a fill, so
    // the light scheme gets a deeper green for the same mark rather than borrowing
    // the dark scheme's answer.
    expect(contrast(dark.stepRule, dark.bg)).toBeGreaterThanOrEqual(3);
    expect(
      contrast(light.stepRule, light.surface),
    ).toBeGreaterThanOrEqual(3);
    expect(contrast(light.stepRule, light.bg)).toBeGreaterThanOrEqual(3);
  });
});

/**
 * The edges.
 *
 * This palette has four weights where the last one had two, and it has them
 * because the *fill* stopped being able to signal anything: in the light scheme a
 * selection is 1.05:1 against the row it sits on, so `--border-active` is the whole
 * signal. A ramp whose order is only in the author's head is not a ramp, so the
 * order is asserted -- and it is the one property here that a diff cannot show.
 */
describe('the edges, which carry what a fill cannot', () => {
  const on = (p: Record<string, string>, key: string, ground: string) => contrast(p[key], ground);

  it('ranks hairline < card edge < panel edge, in both schemes', () => {
    // Measured: dark 1.30 < 1.61 < 1.99, light 1.17 < 1.48 < 1.75. Each step means
    // a different containment -- inside a card, around a card, around a panel --
    // and two of them collapsing to one value is what makes nesting unreadable.
    expect(on(dark, 'line', dark.surface)).toBeLessThan(on(dark, 'border', dark.surface));
    expect(on(dark, 'border', dark.surface)).toBeLessThan(on(dark, 'borderStrong', dark.bg));
    expect(on(light, 'line', light.surface)).toBeLessThan(on(light, 'border', light.surface));
    expect(on(light, 'border', light.surface)).toBeLessThan(on(light, 'borderStrong', light.bg));
  });

  it('keeps a card edge above the hairline it is drawn next to', () => {
    // The failure mode this catches is a card that reads as a divider: an edge
    // indistinguishable from the rule inside it. Measured: 1.61 dark, 1.48 light.
    expect(contrast(dark.border, dark.surface)).toBeGreaterThan(1.4);
    expect(contrast(light.border, light.surface)).toBeGreaterThan(1.4);
  });

  it('keeps the active edge above the 3:1 a control needs', () => {
    // 7.79:1 dark, 3.54:1 light. Light is the tight one and it is the reason the
    // selected fill is so pale: a deeper fill would drop `dim` and `brandInk` under
    // AA on top of it.
    expect(contrast(dark.borderActive, dark.surface)).toBeGreaterThanOrEqual(3);
    expect(contrast(light.borderActive, light.surface)).toBeGreaterThanOrEqual(3);
  });
});

/**
 * The document's own numbers, read out of the document.
 *
 * `docs/design/tokens.md` quotes a contrast ratio for the pairs it tabulates, and until now
 * the quoting ran one way: this file transcribed the numbers it wanted to check, so a table
 * row could disagree with the stylesheet and nothing would notice. Two did, on the day this
 * was written -- `--ink-soft` on the dark surface is 13.04:1 while the table printed 12.73,
 * which is its measurement on `--surface-raised`, and both blocks carry 48 keys while the
 * document still announced 46. A transcription agrees with itself, which is not the same
 * thing as agreeing with the palette.
 *
 * So the rows are scraped from the markdown and computed from `tokens.css` in one run, and
 * nothing in between is hand-written: the token, its two hexes, both ratios and the ground
 * token all come from the document's own line, and every colour comes from the stylesheet.
 * A quoted pair that names no ground is an error rather than a skip, because skipping is how
 * coverage goes missing without anyone deciding to lose it.
 *
 * What this cannot check is stated in the document too: a ratio written as prose instead of
 * as a tabulated pair is nobody's responsibility, and the scraper will not see it appear.
 */

const DOC = import.meta.glob('../../../../docs/design/tokens.md', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

const docText = (): string => {
  const found = Object.entries(DOC).find(([path]) => path.endsWith('docs/design/tokens.md'));
  if (!found) {
    throw new Error(
      `docs/design/tokens.md was not read (files found: ${Object.keys(DOC).join(', ') || 'none'})`,
    );
  }
  return found[1];
};

/** `--ink-soft` -> `inkSoft`, which is the name `named()` gives the same colour. */
const camel = (token: string): string =>
  token.replace(/^--/, '').replace(/-([a-z0-9])/g, (_, c: string) => c.toUpperCase());

type Quoted = {
  /** The document's line, for the failure message. */
  line: string;
  token: string;
  darkHex: string;
  lightHex: string;
  darkRatio: number;
  lightRatio: number;
  ground: string;
};

/** A table row whose first cell names exactly one token. */
const TOKEN_CELL = /^\|\s*`(--[a-z][-\w]*)`\s*\|/;
const HEX_CELL = /^`#([0-9a-fA-F]{6})`$/;
const RATIO_PAIR = /(\d+\.\d+)\s*\/\s*(\d+\.\d+)/;
const GROUND = /on `(--[-\w]+)`/;

/** Every tabulated `dark / light` ratio pair, in the order the columns give it. */
const quotedPairs = (): Quoted[] =>
  docText()
    .split('\n')
    .reduce<Quoted[]>((rows, line) => {
      const token = line.match(TOKEN_CELL)?.[1];
      if (!token) return rows;
      const cells = line.split('|').map((cell) => cell.trim());
      const darkHex = cells[2]?.match(HEX_CELL)?.[1];
      const lightHex = cells[3]?.match(HEX_CELL)?.[1];
      const pair = line.match(RATIO_PAIR);
      if (!darkHex || !lightHex || !pair) return rows;
      rows.push({
        line,
        token,
        darkHex: `#${darkHex}`,
        lightHex: `#${lightHex}`,
        darkRatio: Number(pair[1]),
        lightRatio: Number(pair[2]),
        ground: line.match(GROUND)?.[1] ?? '',
      });
      return rows;
    }, []);

/** The `--`-prefixed declarations one scheme block carries. */
const declaredKeys = (name: 'dark' | 'light'): string[] =>
  Object.keys(scheme(name)).filter((key) => key.startsWith('--'));

/**
 * Every way one quoted row can disagree with the palette: a stale hex, a ratio the colours
 * do not produce, a token or ground the stylesheet never declared.
 *
 * Pure, so the same check runs over the document and over a row invented below to prove the
 * check can fail.
 */
const check = (row: Quoted): string[] => {
  const key = camel(row.token);
  const groundKey = camel(row.ground);
  if (!(key in dark) || !(key in light)) return [`${row.token} is declared by neither scheme`];
  if (!row.ground) return [`${row.token}'s quoted pair names no ground`];
  if (!(groundKey in dark) || !(groundKey in light)) {
    return [`the ground ${row.ground} ${row.token} is quoted on is not a declared token`];
  }
  const problems: string[] = [];
  if (dark[key] !== row.darkHex) {
    problems.push(`${row.token} is ${dark[key]} in dark; the document writes ${row.darkHex}`);
  }
  if (light[key] !== row.lightHex) {
    problems.push(`${row.token} is ${light[key]} in light; the document writes ${row.lightHex}`);
  }
  const darkActual = contrast(dark[key], dark[groundKey]);
  const lightActual = contrast(light[key], light[groundKey]);
  if (!agrees(darkActual, row.darkRatio)) {
    problems.push(
      `${row.token} on ${row.ground} measures ${darkActual.toFixed(2)}:1 in dark; the document quotes ${row.darkRatio}`,
    );
  }
  if (!agrees(lightActual, row.lightRatio)) {
    problems.push(
      `${row.token} on ${row.ground} measures ${lightActual.toFixed(2)}:1 in light; the document quotes ${row.lightRatio}`,
    );
  }
  return problems;
};

describe('the document quotes the palette rather than a memory of it', () => {
  it('has a document to read, and rows worth reading', () => {
    // The two floors this test would rather not need: a glob that silently reads nothing is
    // a green suite that checks nothing, and so is a document whose tables went prose.
    expect(docText()).toContain('# Design tokens');
    const rows = quotedPairs();
    expect(rows.length).toBeGreaterThanOrEqual(10);
  });

  it('agrees with the stylesheet on every row', () => {
    expect(quotedPairs().flatMap(check)).toEqual([]);
  });

  it('names a ground for every pair it quotes', () => {
    // A pair with no ground is not unmeasured, it is unfalsifiable: any number passes.
    expect(quotedPairs().filter((row) => !row.ground).map((row) => row.line)).toEqual([]);
  });

  it('announces the key count the blocks really carry', () => {
    // The document says "the same N keys, and the count is asserted". Both halves are
    // checked: that it still says it, and that the number it says is the number there are.
    const claim = docText().match(/the same (\d+) keys/);
    expect(claim, 'the document should keep stating the key count').not.toBeNull();
    const n = Number(claim?.[1]);
    expect(declaredKeys('dark')).toHaveLength(n);
    expect(declaredKeys('light')).toHaveLength(n);
  });

  it('can fail, which is the whole reason it exists', () => {
    const rows = quotedPairs();
    // Same row, one ratio moved by a rounding nobody would question.
    const first = rows[0];
    expect(check({ ...first, darkRatio: first.darkRatio + 0.2 })).not.toEqual([]);
    // A hex the palette no longer has, which is the change a reviewer cannot see in a diff.
    expect(check({ ...first, darkHex: '#000000' })).not.toEqual([]);
    // A pair whose ground was never declared.
    expect(check({ ...first, ground: '--nonsense' })).not.toEqual([]);
    // And dark/light transposed, the error the column order makes easy to write. Every row
    // here is measurable differently in the two schemes, so all of them must be caught.
    const distinguishable = rows.filter((row) => Math.abs(row.darkRatio - row.lightRatio) > 0.1);
    expect(distinguishable.length).toBeGreaterThanOrEqual(8);
    for (const row of distinguishable) {
      expect(
        check({ ...row, darkRatio: row.lightRatio, lightRatio: row.darkRatio }),
        `a swapped ${row.token} should not read as correct`,
      ).not.toEqual([]);
    }
  });
});

/**
 * The same kind of claim, made a second time inside the stylesheet.
 *
 * `tokens.css` justifies its colours in comments that quote a ratio -- `5.48:1 on surface`,
 * `1.07:1 vs bg` -- and those numbers are the third copy of a fact this file already holds
 * twice (once in the table above, once in the document below it). They drift the same way the
 * document drifted: `--surface-raised` in light is 1.07:1 against `--surface` and 1.12:1
 * against `--bg`, and its comment had attached the smaller number to the larger gap. Nothing
 * noticed, because a comment cannot fail a build.
 *
 * So a comment claim is parsed, resolved to a declared token, and recomputed. The shape is
 * the boundary and it is worth stating plainly: only `<ratio>:1 on|vs <ground>` is read. A
 * figure written any other way -- "the white ink reads 1.42:1 and even `--muted` only reaches
 * 4.40:1", or the `2.74:1` beside `--scroll-thumb` that never says what it is measured on --
 * is invisible here, and those are claims, not measurements.
 */

type Claim = {
  scheme: 'dark' | 'light';
  /** The custom property the comment belongs to. */
  token: string;
  ratio: number;
  /** Whatever the comment called the ground, in its own words. */
  ground: string;
};

/** Words the comments use instead of naming the custom property. */
const GROUND_WORDS: Record<string, string> = {
  bg: '--bg',
  surface: '--surface',
  raised: '--surface-raised',
  acid: '--brand-acid',
  'the dim fill': '--brand-acid-dim',
  'success-bg': '--success-bg',
  'info-bg': '--info-bg',
  'warn-bg': '--warn-bg',
};

/**
 * Which declaration a comment describes: the one it follows on the same line, or, when it
 * stands on its own, the one it introduces.
 */
const claimsIn = (name: 'dark' | 'light'): Claim[] => {
  const text = body(name);
  const declarations = [...text.matchAll(/(--[-\w]+)\s*:\s*[^;]+;/g)].map((m) => ({
    token: m[1],
    start: m.index ?? 0,
    end: (m.index ?? 0) + m[0].length,
  }));
  const claims: Claim[] = [];
  for (const comment of text.matchAll(/\/\*([\s\S]*?)\*\//g)) {
    const at = comment.index ?? 0;
    const sameLine = declarations.find(
      (d) => d.end <= at && !text.slice(d.end, at).includes('\n'),
    );
    const owner = sameLine ?? declarations.find((d) => d.start > at);
    if (!owner) continue;
    for (const claim of comment[1].matchAll(/(\d+\.\d+):1 (?:on|vs) ([^,.;\n:]+)/g)) {
      // `-- = ` is how a comment adds a aside ("on the dim fill -- = `--muted`"), so the
      // ground ends where the dash-dash begins.
      const ground = claim[2].trim().split(' -- ')[0];
      claims.push({ scheme: name, token: owner.token, ratio: Number(claim[1]), ground });
    }
  }
  return claims;
};

/** A ground as a colour: a declared token, a literal hex, or nothing. */
const groundColour = (claim: Claim): string | null => {
  const words = claim.ground.replace(/`/g, '').trim();
  if (/^#[0-9a-f]{6}$/i.test(words)) return words;
  const token = words.startsWith('--')
    ? words
    : words === 'its own fill'
      ? claim.token.endsWith('-ink')
        ? `${claim.token.slice(0, -4)}-bg`
        : ''
      : (GROUND_WORDS[words] ?? '');
  if (!token) return null;
  const key = camel(token);
  const palette = claim.scheme === 'dark' ? dark : light;
  return key in palette ? palette[key] : null;
};

/** The one failure mode a resolved colour still has: the ratio itself. */
const claimProblem = (claim: Claim): string | null => {
  const palette = claim.scheme === 'dark' ? dark : light;
  const key = camel(claim.token);
  if (!(key in palette)) return `${claim.token} in ${claim.scheme} is not a declared token`;
  const ground = groundColour(claim);
  if (!ground) {
    return `the ground "${claim.ground}" of the ${claim.scheme} ${claim.token} claim is not resolvable`;
  }
  const actual = contrast(palette[key], ground);
  return agrees(actual, claim.ratio)
    ? null
    : `${claim.scheme} ${claim.token} on ${ground} measures ${actual.toFixed(2)}:1, the comment says ${claim.ratio}`;
};

describe('the stylesheet cannot quote a ratio its own colours deny', () => {
  const claims = [...claimsIn('dark'), ...claimsIn('light')];

  it('finds the claims it means to read', () => {
    // The floor is the difference between a gate and a no-op: if the parsing above ever
    // stops matching the comment style, this fails instead of passing on nothing.
    expect(claims.length).toBeGreaterThanOrEqual(30);
    expect(claims.filter((claim) => claim.scheme === 'dark').length).toBeGreaterThanOrEqual(15);
  });

  it('resolves every ground it found', () => {
    expect(claims.map(claimProblem).filter((problem) => problem !== null)).toEqual([]);
  });

  it('can fail', () => {
    const claim = claims.find((c) => c.ground === '--surface') ?? claims[0];
    expect(claimProblem({ ...claim, ratio: claim.ratio + 0.3 })).not.toBeNull();
    expect(claimProblem({ ...claim, ground: 'the far side of the ramp' })).not.toBeNull();
    expect(claimProblem({ ...claim, token: '--not-a-token' })).not.toBeNull();
  });
});

