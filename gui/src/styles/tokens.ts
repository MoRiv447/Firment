/**
 * The design tokens, TypeScript side.
 *
 * The GUI has no stylesheet: `gui/src` contains zero `.css` files, and every
 * colour is either an inline `style` or an antd theme token. So a `.css`
 * variable file would be invisible to antd's components -- the token layer has
 * to exist as data and be mapped into `ConfigProvider`, which `antdTheme()`
 * below does.
 *
 * Source of truth: docs/design/tokens.md. Keep the two in step; the values are
 * also duplicated into web/src/styles/tokens.css for the web client, because
 * the two surfaces must not drift into different greys.
 *
 * # Two palettes, one namespace
 *
 * `dark` and `light` below hold the same key set; `Palette` is derived from the
 * dark one, so a key added to one and forgotten in the other is a type error
 * rather than a silent fallback to the wrong grey.
 *
 * `color` reads the **active** palette through getters instead of being a flat
 * object. There are ~160 inline `color.x` reads across the views, and every one
 * of them is evaluated during render, so a getter hands each of them the
 * current scheme without touching a single call site. The alternative -- a
 * context plus a `useTokens()` hook -- would be the same thing with 160 edits,
 * and it would still need a provider to keep `React.memo` from serving stale
 * colours (see `gui/src/lib/theme.ts`).
 *
 * Ordering rule: whoever renders must call `setActivePalette()` **before** its
 * children render, not in an effect, or the first paint is in the old scheme.
 * `App.tsx` does it at the top of the render body.
 */

/** The two schemes. Resolved from `ui.theme` (auto/light/dark) by `theme.ts`. */
export type ThemeMode = 'dark' | 'light';

/**
 * The dark palette. These are the values that shipped, unchanged: adding the
 * light scheme moved no dark value.
 *
 * Where a token is readable as text, the ratio quoted is against `bg`; several
 * were verified against both grounds (docs/design/tokens.md records the ground
 * with every ratio -- #F7F7F5 and #FFFFFF are close enough that a ratio without
 * its ground is ambiguous).
 */
const dark = {
  /** Page background. */
  bg: '#0F0F12',
  /** Cards, panels. */
  surface: '#18181B',
  /** Raised surfaces: popovers, the elevated card. */
  surfaceRaised: '#1F1F23',
  /** Body text. 15.08:1 on `bg`. */
  ink: '#E4E4E7',
  /** Secondary text. Never olive-green: it reads as disabled. 7.47:1 on `bg`. */
  muted: '#A1A1AA',
  /** Hairline dividers and borders. */
  line: '#2A2A2F',
  /** Stronger borders: secondary button outlines. */
  lineStrong: '#3F3F46',

  /**
   * Brand. ONLY for: the logo, the primary CTA fill, progress, the current
   * step. It is ~1.8:1 on a light ground, so it is a highlighter, never a text
   * or icon colour; on a dark ground the text that sits ON it is `onAcid`.
   */
  brandAcid: '#B4F779',
  /** Text/icon colour for content sitting on `brandAcid`. 13.28:1 on it. */
  onAcid: '#15200D',
  /** Brand green dark enough to be readable AS text on a light ground. */
  brandInk: '#3B6D11',

  /**
   * Status green: a different hue from the brand green on purpose (145 deg vs
   * 85 deg). The brand colour is identity, the status colour is feedback; using
   * one for both made "this is Firment" and "this passed" look identical.
   */
  successBg: '#14532D',
  /** 6.49:1 on `successBg`. */
  successInk: '#86EFAC',
  successBorder: '#166534',

  /** Informational accent (branches, links). */
  infoBg: '#1A1E26',
  infoInk: '#7DD3FC',

  /** Warnings and alerts. */
  warnInk: '#EAB308',
  warnBg: '#3F2E06',

  /** Added/removed diff lines. */
  diffAddedBg: '#14311C',
  diffAddedInk: '#86EFAC',
  diffRemovedBg: '#3B1218',
  diffRemovedInk: '#FDA4AF',
  /** Diff hunk headers and context lines. */
  diffMetaInk: '#A1A1AA',

  /**
   * The border around a card, a chip or a control.
   *
   * This used to be `#000000` in BOTH schemes -- a 2px black frame plus a hard
   * offset shadow, the neo-brutalist signature. The visual layer is now
   * neutral: depth comes from a hairline and a soft shadow, so this is a calm
   * grey rather than ink. It keeps its own name because ~60 call sites read it
   * and "the border on this thing" is what they all mean; it no longer carries
   * any brand meaning.
   */
  outline: '#3F3F46',

  /**
   * Elevation. Low and soft, and the shadow is the SECOND separator: two
   * adjacent surfaces are told apart by a hairline first, so these stay subtle
   * enough that a page of cards does not become a pile of floating tiles.
   */
  shadowSm: '0 1px 2px rgba(0,0,0,0.32)',
  shadowMd: '0 4px 12px rgba(0,0,0,0.36)',
  shadowLg: '0 12px 32px rgba(0,0,0,0.44)',

  /**
   * Hover wash for rows and menu items. Unchanged from the value that shipped.
   */
  hover: 'rgba(255,255,255,0.08)',

  /**
   * The three states of a progress step (build / flash / monitor). Derived from
   * the tokens above rather than invented, so a step never introduces a fourth
   * green: done borrows the success pair, "current" is body ink plus the brand
   * rule, pending is muted. Ratios on `bg`: 6.49 / 15.08 / 7.47.
   */
  stepDoneBg: '#14532D',
  stepDoneInk: '#86EFAC',
  stepCurrentInk: '#E4E4E7',
  /** The 2px rule under the current step. Brand colour, so a shape not text. */
  stepRule: '#B4F779',
  stepPendingInk: '#A1A1AA',
  /**
   * A step that actually failed. The one place a step may be red: tokens.md
   * reserves red for a real error, and an unknown outcome is not one. Same
   * family as a removed diff line, because "this broke" and "this was taken
   * out" are the same message at different sizes.
   */
  stepFailedBg: '#3B1218',
  stepFailedInk: '#FDA4AF',

  /**
   * Keyboard focus ring. Acid on a dark ground is 15.06:1, so the brand colour
   * can be its own focus ring here; the light palette cannot do that (1.27:1).
   */
  focusRing: '#B4F779',
} as const;

/**
 * The shape of a palette. Derived from `dark` so the two cannot drift: a key
 * missing from `light` fails the compiler instead of rendering `undefined`.
 */
export type Palette = { [K in keyof typeof dark]: string };

/**
 * The light palette.
 *
 * Two things worth knowing before editing:
 *
 * 1. **A ratio is meaningless without its ground.** `ink` is 17.72:1 on
 *    `surface` and 16.52:1 on `bg`; `muted` is 4.83 / 4.51. Both grounds are in
 *    use, so the pairs that matter are recorded in docs/design/tokens.md.
 * 2. **`brandAcid` is 1.27:1 here.** It is a fill and never text; anything
 *    green-and-readable on light uses `brandInk` (6.21:1), and the focus ring
 *    is `brandInk` for the same reason -- an acid ring is invisible to a
 *    keyboard user. The dark palette can afford the acid ring; this one cannot.
 */
const light: Palette = {
  bg: '#F7F7F5',
  /** 1.07:1 against `bg` -- separation comes from the hairline, not the fill. */
  surface: '#FFFFFF',
  /** Distinguished by border and shadow, not by a lighter fill. */
  surfaceRaised: '#FFFFFF',
  /** 17.72:1 on `surface`, 16.52:1 on `bg`. */
  ink: '#18181B',
  /** 4.83:1 on `surface`, 4.51:1 on `bg` (the tighter of the two). */
  muted: '#71717A',
  /** Hairline. 1.18:1 on `bg`: a line, not the 3:1 non-text threshold. */
  line: '#E4E4E7',
  /** Secondary button outline. 1.48:1 on `surface`. */
  lineStrong: '#D4D4D8',

  brandAcid: '#B4F779',
  /** 13.28:1 on `brandAcid`. */
  onAcid: '#15200D',
  /** 6.21:1 on `surface`, 5.79:1 on `bg`. Green text on light uses this. */
  brandInk: '#3B6D11',

  successBg: '#DCFCE7',
  /** 5.02:1 on `surface`, 4.57:1 on `successBg`. */
  successInk: '#15803D',
  successBorder: '#BBF7D0',

  /**
   * Derived, not given: the table specified the inks but no grounds for them.
   * Chosen to sit in the same family as the status colours, and both pairs were
   * measured -- 5.17:1 and 4.51:1 respectively on their own fills.
   */
  infoBg: '#E0F2FE',
  /** 5.93:1 on `surface`. The dark theme's #7DD3FC is ~2:1 here. */
  infoInk: '#0369A1',
  warnBg: '#FEF3C7',
  /** 5.02:1 on `surface`; the dark theme's #EAB308 is 1.9:1 here. */
  warnInk: '#B45309',

  diffAddedBg: '#DCFCE7',
  /** 4.57:1 on `diffAddedBg`. */
  diffAddedInk: '#15803D',
  diffRemovedBg: '#FEE2E2',
  /** 6.56:1 on `diffRemovedBg`. */
  diffRemovedInk: '#9F1239',
  /** 4.51:1 on `bg`. */
  diffMetaInk: '#71717A',

  /**
   * Matches the dark scheme's role, not its value: the neutral system separates
   * with a hairline on both grounds, so this is `lineStrong`'s grey. It used to
   * stay black in both schemes -- a thick black frame on a light ground reads as
   * heavy rather than deliberate, which is exactly why this changed.
   */
  outline: '#D4D4D8',

  /**
   * Softer and wider than the dark scheme's: a black shadow on a white ground
   * reads as dirt, so these are large-radius and very low alpha.
   */
  shadowSm: '0 1px 2px rgba(16,24,40,0.06)',
  shadowMd: '0 4px 12px rgba(16,24,40,0.08)',
  shadowLg: '0 12px 32px rgba(16,24,40,0.12)',

  /**
   * Pale acid wash. Every neutral tried here fails: `muted` is 4.51:1 on `bg`
   * and that ground is already the floor, so a grey hover pulls it under AA
   * (4.40:1 at #F4F4F5, 4.47:1 at #F6F6F7). This tint is the only candidate
   * that holds -- 4.68:1 for `muted`, 17.16:1 for `ink` -- and it reads as a
   * weaker sibling of the solid-acid selection rather than competing with it.
   */
  hover: '#F6FEEF',

  /** 6.19:1 (done) / 16.52:1 (current) / 4.51:1 (pending) on `bg`. */
  stepDoneBg: '#EAF3DE',
  stepDoneInk: '#3F6212',
  stepCurrentInk: '#18181B',
  stepRule: '#B4F779',
  stepPendingInk: '#6B7280',
  /** 6.56:1 on `stepFailedBg`, mirrored from the removed-diff pair above. */
  stepFailedBg: '#FEE2E2',
  stepFailedInk: '#9F1239',

  /** 6.21:1. NOT the acid: that is 1.27:1 and a keyboard user cannot see it. */
  focusRing: '#3B6D11',
};

const palettes: Record<ThemeMode, Palette> = { dark, light };

let activeMode: ThemeMode = 'dark';

/**
 * Point every `color.x` getter at `mode`.
 *
 * Call this during render, before children render -- an effect is one frame too
 * late and paints the previous scheme.
 */
export function setActivePalette(mode: ThemeMode): void {
  activeMode = mode;
}

/** The palette for an explicit mode, for code that must not read the global. */
export function paletteFor(mode: ThemeMode): Palette {
  return palettes[mode];
}

/**
 * The active palette, read through getters.
 *
 * A getter object rather than a flat `as const`: the reads happen inside render
 * functions, so they pick up the scheme `setActivePalette` set for the pass that
 * is currently running.
 */
export const color: Palette = Object.keys(dark).reduce((acc, key) => {
  Object.defineProperty(acc, key, {
    get: () => palettes[activeMode][key as keyof Palette],
    enumerable: true,
  });
  return acc;
}, {} as Record<string, unknown>) as Palette;

/** Font stacks. Code and UI are separate on purpose -- the UI is not a
 * terminal, and setting prose in a monospace was making every label shout. */
export const font = {
  /** UI text. */
  sans: "'Inter', 'Noto Sans SC', system-ui, -apple-system, 'Segoe UI', sans-serif",
  /** Code, diffs, logs, ids. */
  mono: "'JetBrains Mono', 'Noto Sans SC', 'Cascadia Code', Consolas, monospace",
} as const;

/** Corner radii. A badge is not a card, so there are four tiers and no bare
 *  numbers anywhere else in the tree (`no-literal-tokens.test.ts` enforces it).
 *
 *  These used to be 0 / 2 / 4 / 8, with 0 reserved for the logo and every icon
 *  tile -- hard right angles as part of the neo-brutalist frame. The neutral
 *  system rounds instead: a 0 on a card reads as an unfinished box once the
 *  black outline around it is gone. */
export const radius = {
  /** Logo and icon tiles, and the cards built on the same shape. */
  tile: 8,
  /** Badges, chips, tooltips. */
  chip: 4,
  /** Inputs and buttons. */
  control: 6,
  /** Cards and panels. */
  panel: 12,
} as const;

/**
 * The slant -- the one signature that cannot be substituted.
 *
 * Geometry lives here so no component invents its own numbers, and so the
 * optical correction stays attached to the reason for it.
 */
export const slant = {
  /** Horizontal run of the cut, in px. */
  cut: 12,
  /** The gap between adjacent slanted edges, in px. */
  gap: 8,
  /**
   * Extra left padding over the right, in px.
   *
   * The cut removes a triangle from one side, which moves the remaining shape's
   * centre of mass ~2.5px the other way; without this the label reads as
   * off-centre even though it is geometrically centred.
   */
  opticalPadLeft: 5,
  /** Every control on a row is this tall, so colour carries the hierarchy. */
  controlHeight: 40,
} as const;

/** Motion. Enter 160 / standard 200 / exit 120. */
export const motion = {
  ease: 'cubic-bezier(.2,.8,.2,1)',
  enter: 160,
  standard: 200,
  exit: 120,
  stagger: 40,
} as const;

/**
 * The antd theme, in both light and dark form.
 *
 * Kept as a function rather than a constant because antd's own `token` needs
 * the raw values above, and because a `ui.theme = light|dark|auto` setting
 * flips this rather than forking it.
 *
 * What actually differs between the modes:
 *
 * * **Grounds, text and borders** come from the mode's palette, and the border
 *   mapping is the same in both: a hairline `line`, with `outline` for anything
 *   that needs to read as a control edge. The old code framed every dark-mode
 *   control in `#000000` and dropped to a hairline only in light -- which is
 *   what made the two schemes look like different products.
 * * **Elevation** comes from `shadowSm`, not from a hard offset block.
 * * `colorSuccess` is `successInk` in **both** modes. The previous code fed the
 *   light mode `brandInk`, which would have painted "this passed" in the brand
 *   green -- the exact confusion the 85deg/145deg split exists to prevent.
 * * `colorError` is `diffRemovedInk` in both modes, which is where the two
 *   hardcoded values (a dark pink, a light maroon) already lived.
 */
export function antdTheme(mode: ThemeMode = 'dark') {
  const p = paletteFor(mode);
  return {
    token: {
      colorPrimary: p.brandAcid,
      // antd derives hover/active from the primary; on a dark ground the
      // derived shades of a bright acid green land where we want them.
      colorBgLayout: p.bg,
      colorBgContainer: p.surface,
      colorBgElevated: p.surfaceRaised,
      colorText: p.ink,
      colorTextSecondary: p.muted,
      colorBorder: p.outline,
      colorBorderSecondary: p.line,
      colorSuccess: p.successInk,
      colorError: p.diffRemovedInk,
      colorWarning: p.warnInk,
      borderRadius: radius.control,
      borderRadiusLG: radius.panel,
      borderRadiusSM: radius.chip,
      // Popovers, dropdowns and modals float; a hairline is not enough once a
      // surface overlaps live content, so these get the real elevation.
      boxShadow: p.shadowMd,
      boxShadowSecondary: p.shadowLg,
      fontFamily: font.sans,
      // Not a theme token upstream, but antd reads it for code-ish text when a
      // component asks for the mono family.
      fontFamilyCode: font.mono,
    },
    components: {
      Menu: {
        itemBg: 'transparent',
        itemSelectedBg: p.brandAcid,
        itemSelectedColor: p.onAcid,
        itemHoverBg: p.hover,
        // Was 0 with the rest of the neo-brutalist frame; the selected tab is
        // the one item where the tier has to be visible, so it is `chip`.
        itemBorderRadius: radius.chip,
      },
      Card: { headerBg: 'transparent' },
      // The acid fill carries `onAcid` text, never white-on-green: white on
      // #B4F779 is 1.3:1.
      Button: { fontWeight: 600, primaryColor: p.onAcid },
      Tag: { borderRadiusSM: radius.chip, borderRadiusLG: radius.chip },
    },
  };
}
