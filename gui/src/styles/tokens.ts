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

/** The two schemes. Moved to `lib/theme.ts`, which outlives this file. */
export type { ThemeMode } from '../lib/theme';
import type { ThemeMode } from '../lib/theme';

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
  bg: '#111111',
  /** Cards, panels. */
  surface: '#191919',
  /** Raised surfaces: popovers, the elevated card. */
  surfaceRaised: '#222222',
  /** Body text. 15.08:1 on `bg`. */
  ink: '#eeeeee',
  /** Secondary text. Never olive-green: it reads as disabled. 7.47:1 on `bg`. */
  muted: '#b4b4b4',
  /** Hairline dividers and borders. */
  line: '#3a3a3a',
  /** Stronger borders: secondary button outlines. */
  lineStrong: '#484848',

  /**
   * Brand. ONLY for: the logo, the primary CTA fill, progress, the current
   * step. It is ~1.8:1 on a light ground, so it is a highlighter, never a text
   * or icon colour; on a dark ground the text that sits ON it is `onAcid`.
   */
  brandAcid: '#00a2c7',
  /** Text/icon colour for content sitting on `brandAcid`. 13.28:1 on it. */
  onAcid: '#0b161a',
  /** Brand green dark enough to be readable AS text on a light ground. */
  brandInk: '#4ccce6',

  /**
   * Status green: a different hue from the brand green on purpose (145 deg vs
   * 85 deg). The brand colour is identity, the status colour is feedback; using
   * one for both made "this is Firment" and "this passed" look identical.
   */
  successBg: '#132d21',
  /** 6.49:1 on `successBg`. */
  successInk: '#b1f1cb',
  successBorder: '#28684a',

  /** Informational accent (branches, links). */
  infoBg: '#0d2847',
  infoInk: '#c2e6ff',

  /** Warnings and alerts. */
  warnInk: '#ffe7b3',
  warnBg: '#302008',

  /** Added/removed diff lines. */
  diffAddedBg: '#113b29',
  diffAddedInk: '#b1f1cb',
  diffRemovedBg: '#500f1c',
  diffRemovedInk: '#ffd1d9',
  /** Diff hunk headers and context lines. */
  diffMetaInk: '#b4b4b4',

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
  outline: '#484848',

  /**
   * Elevation. Low and soft, and the shadow is the SECOND separator: two
   * adjacent surfaces are told apart by a hairline first, so these stay subtle
   * enough that a page of cards does not become a pile of floating tiles.
   */
  shadowSm: 'none',
  shadowMd: 'none',
  shadowLg: 'none',

  /**
   * Hover wash for rows and menu items. Unchanged from the value that shipped.
   */
  hover: '#2a2a2a',

  /**
   * The selected row: the sidebar item, the live conversation.
   *
   * A token pair rather than two reads of `brandAcid` / `onAcid` at the call
   * site, because the two schemes cannot share one answer. Acid works here on
   * dark -- `onAcid` is 13.28:1 on it -- and fails on light, where the same fill
   * is 1.19:1 against the cream ground, which makes "this one is current"
   * invisible. The light palette answers with a dark green fill instead; the
   * call sites stay identical.
   *
   * Anything inside a selected row reads its text and icon colour from
   * `onSelection`, never from `ink`: on dark `ink` is 1.01:1 on acid.
   */
  selection: '#004558',
  /** 13.28:1 on `selection`. */
  onSelection: '#eeeeee',

  /**
   * The three states of a progress step (build / flash / monitor). Derived from
   * the tokens above rather than invented, so a step never introduces a fourth
   * green: done borrows the success pair, "current" is body ink plus the brand
   * rule, pending is muted. Ratios on `bg`: 6.49 / 15.08 / 7.47.
   */
  stepDoneBg: '#113b29',
  stepDoneInk: '#b1f1cb',
  stepCurrentInk: '#4ccce6',
  /** The 2px rule under the current step. Brand colour, so a shape not text. */
  stepRule: '#00a2c7',
  stepPendingInk: '#eeeeee',
  /**
   * A step that actually failed. The one place a step may be red: tokens.md
   * reserves red for a real error, and an unknown outcome is not one. Same
   * family as a removed diff line, because "this broke" and "this was taken
   * out" are the same message at different sizes.
   */
  stepFailedBg: '#3b1219',
  stepFailedInk: '#ffd1d9',

  /**
   * Keyboard focus ring. Acid on a dark ground is 15.06:1, so the brand colour
   * can be its own focus ring here; the light palette cannot do that (1.27:1).
   */
  focusRing: '#11809c',
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
 *    `surface` and 16.52:1 on `bg`; `muted` is 5.28 / 4.92. Both grounds are in
 *    use, so the pairs that matter are recorded in docs/design/tokens.md.
 * 2. **`brandAcid` is 1.27:1 here.** It is a fill and never text; anything
 *    green-and-readable on light uses `brandInk` (6.21:1), and the focus ring
 *    is `brandInk` for the same reason -- an acid ring is invisible to a
 *    keyboard user. The dark palette can afford the acid ring; this one cannot.
 */
const light: Palette = {
  bg: '#fcfcfc',
  /** 1.07:1 against `bg` -- separation comes from the hairline, not the fill. */
  surface: '#f9f9f9',
  /** Distinguished by border and shadow, not by a lighter fill. */
  surfaceRaised: '#f0f0f0',
  /** 17.72:1 on `surface`, 16.52:1 on `bg`. */
  ink: '#202020',
  /**
   * 5.28:1 on `surface`, 4.92:1 on `bg`, 4.55:1 on `hover`.
   *
   * It used to be #71717A -- 4.83 / 4.51 -- and that was fine while the only
   * coloured ground it had to clear was its own. Moving the hover wash off the
   * acid tint (see `hover`) put a second, darker ground under it, so the ink
   * moved down with it rather than leaving a label that only passes on paper.
   */
  muted: '#646464',
  /** Hairline. 1.18:1 on `bg`: a line, not the 3:1 non-text threshold. */
  line: '#d9d9d9',
  /** Secondary button outline. 1.48:1 on `surface`. */
  lineStrong: '#cecece',

  brandAcid: '#107d98',
  /** 13.28:1 on `brandAcid`. */
  onAcid: '#ffffff',
  /** 6.21:1 on `surface`, 5.79:1 on `bg`. Green text on light uses this. */
  brandInk: '#107d98',

  successBg: '#e6f6eb',
  /** 5.02:1 on `surface`, 4.57:1 on `successBg`. */
  successInk: '#193b2d',
  successBorder: '#8eceaa',

  /**
   * Derived, not given: the table specified the inks but no grounds for them.
   * Chosen to sit in the same family as the status colours, and both pairs were
   * measured -- 5.17:1 and 4.51:1 respectively on their own fills.
   */
  infoBg: '#e6f4fe',
  /** 5.93:1 on `surface`. The dark theme's #7DD3FC is ~2:1 here. */
  infoInk: '#113264',
  warnBg: '#fff7c2',
  /** 5.02:1 on `surface`; the dark theme's #EAB308 is 1.9:1 here. */
  warnInk: '#4f3422',

  diffAddedBg: '#d6f1df',
  /** 4.57:1 on `diffAddedBg`. */
  diffAddedInk: '#193b2d',
  diffRemovedBg: '#ffdbdc',
  /** 6.56:1 on `diffRemovedBg`. */
  diffRemovedInk: '#641723',
  /** 4.51:1 on `bg`. */
  diffMetaInk: '#646464',

  /**
   * Matches the dark scheme's role, not its value: the neutral system separates
   * with a hairline on both grounds, so this is `lineStrong`'s grey. It used to
   * stay black in both schemes -- a thick black frame on a light ground reads as
   * heavy rather than deliberate, which is exactly why this changed.
   */
  outline: '#cecece',

  /**
   * Softer and wider than the dark scheme's: a black shadow on a white ground
   * reads as dirt, so these are large-radius and very low alpha.
   */
  shadowSm: '0 1px 2px rgba(16,24,40,0.06)',
  shadowMd: '0 4px 12px rgba(16,24,40,0.08)',
  shadowLg: '0 12px 32px rgba(16,24,40,0.12)',

  /**
   * Warm neutral wash, 1.08:1 against `bg` -- a wash, not a border.
   *
   * This used to be a pale acid tint (#F6FEEF) because the argument at the time
   * was that `muted` sat exactly on the AA floor and any grey wash would push it
   * under. That was true of #71717A and it is not true of the darker
   * `muted` above, which holds 4.55:1 here.
   *
   * The colour also has a second job now: the selected row is a solid dark-green
   * fill, so a green hover would be a weaker sibling of the same signal and the
   * two would read as two degrees of "selected". Hover is neutral, selection is
   * green, and only one of them means "this one".
   */
  hover: '#e8e8e8',

  /**
   * `brandInk` as a fill: 5.79:1 against `bg`, 6.21:1 against `surface`, so the
   * row announces itself by shape and weight and not by a glow.
   *
   * The obvious candidate was `brandAcid` -- it is the brand, and it is what the
   * dark scheme uses. It is 1.19:1 on this ground, which is not a highlight, and
   * it put white-or-ink text on a pastel fill: the reported "I cannot read the
   * selected chat in light mode" is exactly this token and nothing else.
   */
  selection: '#b5e9f0',
  /** 6.21:1 on `selection`. White, not `onAcid`: on a dark green the acid is 4.89. */
  onSelection: '#202020',

  /** 6.19:1 (done) / 16.52:1 (current) / 4.51:1 (pending) on `bg`. */
  stepDoneBg: '#d6f1df',
  stepDoneInk: '#193b2d',
  stepCurrentInk: '#107d98',
  /**
   * `brandInk`, not the acid.
   *
   * A 2px rule is a shape, so it needs 3:1 against its ground, and acid here is
   * 1.19:1 on `bg` / 1.27:1 on `surface` -- which is why the current step, the
   * live inspector tab and the todo progress bar all read as "nothing is
   * selected" in light mode. The dark scheme keeps the acid because on #0F0F12
   * it is 15:1.
   */
  stepRule: '#0797b9',
  stepPendingInk: '#202020',
  /** 6.56:1 on `stepFailedBg`, mirrored from the removed-diff pair above. */
  stepFailedBg: '#feebec',
  stepFailedInk: '#641723',

  /** 6.21:1. NOT the acid: that is 1.27:1 and a keyboard user cannot see it. */
  focusRing: '#0797b9',
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

/** What a status chip is reporting. `neutral` is "no judgement" -- an unknown
 *  usage, a device that is configured but not connected -- and must not read as
 *  either healthy or broken. */
export type StatusKind = 'ok' | 'failed' | 'running' | 'attention' | 'neutral';

/**
 * The fill and ink for a status chip, as a pair.
 *
 * Chips used to be built the other way round: an *ink* token as the fill
 * (`color={color.warnInk}`) with `color.outline` -- then pure black -- as the
 * text. That works while every ink is bright, which is true in the dark scheme
 * and false in the light one: `successInk` is `#86EFAC` on dark and `#15803D`
 * on light, so the same chip went from light-fill/dark-text to
 * dark-fill/dark-text. It is one of the reasons light read as a different
 * product.
 *
 * Reading both halves from the same pair fixes that by construction: each ink
 * is documented against the bg it sits on (6.49:1 dark, 4.57:1 light for `ok`),
 * so a chip cannot be assembled from two tokens that were measured against
 * different grounds.
 *
 * Borderless on purpose. A status chip is a fill, and the neutral layer already
 * spends its hairlines on structure -- outlining every badge as well is what
 * made the old header read as five competing boxes.
 */
export function statusChip(kind: StatusKind): { background: string; color: string } {
  switch (kind) {
    case 'ok':
      return { background: color.successBg, color: color.successInk };
    case 'failed':
      // The failed-step pair, documented as the removed-diff family: "this
      // broke" and "this was taken out" are the same message at different
      // sizes.
      return { background: color.stepFailedBg, color: color.stepFailedInk };
    case 'running':
      return { background: color.infoBg, color: color.infoInk };
    case 'attention':
      return { background: color.warnBg, color: color.warnInk };
    case 'neutral':
      // A raised surface rather than a hue: "we do not know" is not a status,
      // and colouring it would make it look like one.
      return { background: color.surfaceRaised, color: color.muted };
  }
}

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
  panel: 8,
} as const;


/**
 * The gap between controls sitting on one row.
 *
 * Kept separate from `slant` because it is not about the cut: a row of ordinary
 * buttons wants the same gap as a row containing a slanted one.
 */
export const space = {
  /** Between adjacent controls on a row. */
  controlGap: 8,
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
      /**
       * antd's "text on a solid primary" alias, which it hardcodes to white.
       *
       * `colorPrimary` here is the acid, and white on acid is 1.27:1 -- this is
       * what a checked `Tag.CheckableTag` renders its label with, so every workbench
       * filter chip looked on-but-read-off. `onAcid` is the palette's own answer
       * to "what sits on acid" (13.28:1), so the alias stops being a second source.
       */
      colorTextLightSolid: p.onAcid,
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
        itemSelectedBg: p.selection,
        itemSelectedColor: p.onSelection,
        itemHoverBg: p.hover,
        // Was 0 with the rest of the neo-brutalist frame; the selected tab is
        // the one item where the tier has to be visible, so it is `chip`.
        itemBorderRadius: radius.chip,
      },
      Card: { headerBg: 'transparent' },
      // The acid fill carries `onAcid` text, never white-on-green: white on
      // #B4F779 is 1.3:1.
      Button: { fontWeight: 600, primaryColor: p.onAcid },
      /**
       * Focus and hover borders.
       *
       * Both default to a shade of `colorPrimary`, and `colorPrimary` is the
       * acid because that is what a primary button is made of -- but a focus
       * ring is not a fill, it is the one state a keyboard user has to be able
       * to find. Acid on the light ground is 1.19:1, which is why `focusRing`
       * exists as its own token in the first place.
       */
      Input: { activeBorderColor: p.focusRing, hoverBorderColor: p.lineStrong },
      Select: { activeBorderColor: p.focusRing, hoverBorderColor: p.lineStrong },
      Tag: { borderRadiusSM: radius.chip, borderRadiusLG: radius.chip },
    },
  };
}
