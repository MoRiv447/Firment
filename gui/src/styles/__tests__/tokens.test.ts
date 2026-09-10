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
    expect(color.bg).toBe('#0F0F12');
    expect(color.ink).toBe('#E4E4E7');

    setActivePalette('light');
    expect(color.bg).toBe('#F7F7F5');
    expect(color.ink).toBe('#18181B');

    // Getters, not a snapshot: the same object must have changed.
    setActivePalette('dark');
    expect(color.bg).toBe('#0F0F12');
  });
});

describe('text is readable on every ground it is used on', () => {
  // [label, foreground, background, the ratio quoted in tokens.md]
  const cases: Array<[string, string, string, number]> = [
    ['light ink on surface', '#18181B', '#FFFFFF', 17.72],
    ['light ink on bg', '#18181B', '#F7F7F5', 16.52],
    ['light muted on surface', '#71717A', '#FFFFFF', 4.83],
    ['light muted on bg', '#71717A', '#F7F7F5', 4.51],
    ['light brandInk on surface', '#3B6D11', '#FFFFFF', 6.21],
    ['light successInk on surface', '#15803D', '#FFFFFF', 5.02],
    ['light diffAddedInk on diffAddedBg', '#15803D', '#DCFCE7', 4.57],
    ['light diffRemovedInk on diffRemovedBg', '#9F1239', '#FEE2E2', 6.56],
    ['light stepDoneInk on stepDoneBg', '#3F6212', '#EAF3DE', 6.19],
    ['light stepPendingInk on bg', '#6B7280', '#F7F7F5', 4.51],
    ['dark ink on bg', '#E4E4E7', '#0F0F12', 15.08],
    ['dark muted on bg', '#A1A1AA', '#0F0F12', 7.47],
    ['dark successInk on successBg', '#86EFAC', '#14532D', 6.49],
    ['onAcid on brandAcid', '#15200D', '#B4F779', 13.28],
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

  it('keeps the acid green off text duty in the light scheme', () => {
    // The rule the whole two-green split rests on: #B4F779 is a highlighter on
    // a light ground (1.27:1), so it can never carry a label there.
    const acidOnSurface = contrast('#B4F779', '#FFFFFF');
    expect(acidOnSurface).toBeLessThan(2);
    expect(paletteFor('light').brandInk).not.toBe(paletteFor('light').brandAcid);
  });

  it('gives the light scheme a focus ring a keyboard user can see', () => {
    // The acid ring is 1.27:1 on the light ground; the dark scheme can afford
    // it (15.06:1), the light one cannot.
    expect(contrast(paletteFor('light').focusRing, '#FFFFFF')).toBeGreaterThanOrEqual(3);
    expect(paletteFor('light').focusRing).toBe(paletteFor('light').brandInk);
    expect(contrast(paletteFor('dark').focusRing, paletteFor('dark').bg)).toBeGreaterThan(3);
  });

  it('keeps the light hover wash from sinking the muted label', () => {
    // Every neutral hover dark enough to read as a hover pulls `muted` under
    // AA on the light ground (#F4F4F5 gives 4.40). This pins the reason the
    // light hover is an acid tint instead.
    expect(contrast(paletteFor('light').muted, paletteFor('light').hover)).toBeGreaterThanOrEqual(
      AA,
    );
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
      expect(antdTheme(mode).token.colorSuccess).not.toBe(palette.brandAcid);
      expect(antdTheme(mode).token.colorSuccess).not.toBe(palette.brandInk);
    }
  });

  it('defaults to dark, matching the shipped scheme', () => {
    expect(antdTheme().token.colorBgLayout).toBe('#0F0F12');
  });
});
