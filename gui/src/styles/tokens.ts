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
 */

/** Grounds and text. Deliberately neutral: an interface tinted green makes the
 * diff's own red/green harder to read, and Firment's screens are mostly diff. */
export const color = {
  /** Page background. */
  bg: '#0F0F12',
  /** Cards, panels. */
  surface: '#18181B',
  /** Raised surfaces: popovers, the elevated card. */
  surfaceRaised: '#1F1F23',
  /** Body text. */
  ink: '#E4E4E7',
  /** Secondary text. Never olive-green: it reads as disabled. */
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
  /** Text/icon colour for content sitting on `brandAcid`. */
  onAcid: '#15200D',
  /** Brand green dark enough to be readable AS text on a light ground. */
  brandInk: '#3B6D11',

  /**
   * Status green: a different hue from the brand green on purpose (145 deg vs
   * 85 deg). The brand colour is identity, the status colour is feedback; using
   * one for both made "this is Firment" and "this passed" look identical.
   */
  successBg: '#14532D',
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

  /** The one true black: the neo-brutalist outline and hard shadow. */
  outline: '#000000',
} as const;

/** Font stacks. Code and UI are separate on purpose -- the UI is not a
 * terminal, and setting prose in a monospace was making every label shout. */
export const font = {
  /** UI text. */
  sans: "'Inter', 'Noto Sans SC', system-ui, -apple-system, 'Segoe UI', sans-serif",
  /** Code, diffs, logs, ids. */
  mono: "'JetBrains Mono', 'Noto Sans SC', 'Cascadia Code', Consolas, monospace",
} as const;

/** Corner radii. Not one value everywhere: a slanted CTA loses its slant past
 * 8px, while a large panel needs the softness. */
export const radius = {
  /** Logo and icon tiles: hard edges, the brand anchor. */
  brand: 0,
  /** Badges, chips, tooltips. */
  chip: 2,
  /** Inputs and buttons. */
  control: 4,
  /** Cards and panels. */
  panel: 8,
} as const;

/**
 * The antd theme, in both light and dark form.
 *
 * Kept as a function rather than a constant because antd's own `token` needs
 * the raw values above, and because a future `ui.theme = light|dark` setting
 * should flip this rather than fork it.
 */
export function antdTheme(mode: 'dark' | 'light' = 'dark') {
  const dark = mode === 'dark';
  return {
    token: {
      colorPrimary: color.brandAcid,
      // antd derives hover/active from the primary; on a dark ground the
      // derived shades of a bright acid green land where we want them.
      colorBgLayout: color.bg,
      colorBgContainer: color.surface,
      colorBgElevated: color.surfaceRaised,
      colorText: color.ink,
      colorTextSecondary: color.muted,
      colorBorder: color.outline,
      colorBorderSecondary: color.outline,
      colorSuccess: dark ? color.successInk : color.brandInk,
      colorError: dark ? color.diffRemovedInk : '#9F1239',
      colorWarning: color.warnInk,
      borderRadius: radius.control,
      fontFamily: font.sans,
      // Not a theme token upstream, but antd reads it for code-ish text when a
      // component asks for the mono family.
      fontFamilyCode: font.mono,
    },
    components: {
      Menu: {
        itemBg: 'transparent',
        itemSelectedBg: color.brandAcid,
        itemSelectedColor: color.onAcid,
        itemHoverBg: 'rgba(255,255,255,0.08)',
        itemBorderRadius: 0,
      },
      Card: { headerBg: 'transparent' },
      Button: { fontWeight: 600 },
      Tag: { borderRadiusSM: radius.chip, borderRadiusLG: radius.chip },
    },
  };
}
