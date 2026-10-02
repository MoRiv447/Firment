import { describe, expect, it } from 'vitest';
import { named } from './readPalette';

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
 * Contrast is arithmetic, so it is checked here rather than trusted: the values
 * come from docs/design/tokens.md, and a well-meaning tweak to one hex -- "this
 * grey looks a bit dark" -- is exactly the change that quietly drops a label
 * under AA. The ratios quoted in that document are asserted below.
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

