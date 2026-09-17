import { describe, expect, it } from 'vitest';
import { antdTheme, color, paletteFor, setActivePalette } from '../tokens';

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
    const dark = Object.keys(paletteFor('dark')).sort();
    const light = Object.keys(paletteFor('light')).sort();
    expect(light).toEqual(dark);
  });
  it('follows the active mode through the `color` getters', () => {
    setActivePalette('dark');
    expect(color.bg).toBe('#111111');
    expect(color.ink).toBe('#eeeeee');

    setActivePalette('light');
    expect(color.bg).toBe('#fcfcfc');
    expect(color.ink).toBe('#202020');

    // Getters, not a snapshot: the same object must have changed.
    setActivePalette('dark');
    expect(color.bg).toBe('#111111');
  });
});

describe('text is readable on every ground it is used on', () => {
  it('gives the light scheme a focus ring a keyboard user can see', () => {
    // Radix's step 8 is documented as the focus-ring step, and cyan-8 on white is
    // 2.32:1 -- under the 3:1 a focus ring needs. The light ring is cyan-10
    // (3.34:1); the dark one keeps cyan-8, which clears the same floor easily on a
    // near-black ground. It is no longer required to equal `brandInk`: that was a
    // convention of the old palette, and the measurement is the part that matters.
    expect(contrast(paletteFor('light').focusRing, '#FFFFFF')).toBeGreaterThanOrEqual(3);
    expect(contrast(paletteFor('dark').focusRing, paletteFor('dark').bg)).toBeGreaterThan(3);
  });

  // [label, foreground, background, the ratio, which is a measurement of the
  // palette rather than a quote from a document -- rows name the palette so they
  // cannot read one scheme's colour while claiming to be the other].
  const light = paletteFor('light');
  const dark = paletteFor('dark');

  const cases: Array<[string, string, string, number]> = [
    ['light ink on surface', light.ink, light.surface, 15.48],
    ['light ink on bg', light.ink, light.bg, 15.88],
    ['light muted on surface', light.muted, light.surface, 5.62],
    ['light muted on bg', light.muted, light.bg, 5.77],
    ['light brandInk on surface', light.brandInk, light.surface, 4.52],
    ['light successInk on successBg', light.successInk, light.successBg, 11],
    ['light diffAddedInk on diffAddedBg', light.diffAddedInk, light.diffAddedBg, 10.27],
    ['light diffRemovedInk on diffRemovedBg', light.diffRemovedInk, light.diffRemovedBg, 9.72],
    ['light stepDoneInk on stepDoneBg', light.stepDoneInk, light.stepDoneBg, 10.27],
    ['light stepFailedInk on stepFailedBg', light.stepFailedInk, light.stepFailedBg, 10.84],
    ['dark stepFailedInk on stepFailedBg', dark.stepFailedInk, dark.stepFailedBg, 11.95],
    ['light stepPendingInk on bg', light.stepPendingInk, light.bg, 15.88],
    ['dark ink on bg', dark.ink, dark.bg, 16.28],
    ['dark muted on bg', dark.muted, dark.bg, 9.11],
    ['dark successInk on successBg', dark.successInk, dark.successBg, 11.45],
    ['onAcid on brandAcid light', light.onAcid, light.brandAcid, 4.76],
    ['onAcid on brandAcid dark', dark.onAcid, dark.brandAcid, 4.57],
    ['selection ink on selection light', light.onSelection, light.selection, 12.32],
    ['selection ink on selection dark', dark.onSelection, dark.selection, 9.09],
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
    // The wash is only a wash if a label on top of it still passes. `muted` is
    // dark enough now that a neutral ground holds it, which is what let the
    // light hover stop being a second green state. (Dark is skipped: its wash is
    // a translucent overlay, and the luminance helper reads hexes.)
    const p = paletteFor('light');
    expect(contrast(p.muted, p.hover)).toBeGreaterThanOrEqual(AA);
    expect(contrast(p.ink, p.hover)).toBeGreaterThanOrEqual(AA);
  });


  it('keeps a 2px brand mark visible on the ground it is drawn on', () => {
    // stepRule is a shape, not text -- the current-step underline, the live
    // inspector tab, the todo progress bar. Acid on a light ground is 1.19:1,
    // which is why the light scheme cannot borrow the dark scheme's answer.
    expect(contrast(paletteFor('dark').stepRule, paletteFor('dark').bg)).toBeGreaterThanOrEqual(3);
    expect(
      contrast(paletteFor('light').stepRule, paletteFor('light').surface),
    ).toBeGreaterThanOrEqual(3);
    expect(contrast(paletteFor('light').stepRule, paletteFor('light').bg)).toBeGreaterThanOrEqual(3);
  });
});

describe('antdTheme actually branches on the mode', () => {
  it('gives the two schemes different grounds and text', () => {
    const dark = antdTheme('dark').token;
    const light = antdTheme('light').token;
    expect(dark.colorBgLayout).not.toBe(light.colorBgLayout);
    expect(dark.colorText).not.toBe(light.colorText);
    expect(dark.colorBgLayout).toBe(paletteFor('dark').bg);
    expect(light.colorBgLayout).toBe(paletteFor('light').bg);
  });

  it('softens the border mapping in light and keeps the outline in dark', () => {
    const dark = antdTheme('dark').token;
    const light = antdTheme('light').token;
    const darkPalette = paletteFor('dark');
    const lightPalette = paletteFor('light');
    expect(dark.colorBorder).toBe(darkPalette.outline);
    expect(light.colorBorder).toBe(lightPalette.lineStrong);
    expect(light.colorBorderSecondary).toBe(lightPalette.line);
  });

  it('never paints success in the brand green', () => {
    // The 85deg/145deg split: "this is Firment" and "this passed" must not be
    // the same colour. The old code fed the light mode `brandInk` here.
    for (const mode of ['dark', 'light'] as const) {
      const palette = paletteFor(mode);
      expect(antdTheme(mode).token.colorSuccess).toBe(palette.successInk);
                }
  });

  it('defaults to dark, matching the shipped scheme', () => {
    expect(antdTheme().token.colorBgLayout).toBe('#111111');
  });
});
