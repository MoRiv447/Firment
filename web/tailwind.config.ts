import type { Config } from "tailwindcss";

/**
 * Every colour name here resolves to a design token, so a class like
 * `bg-surface` or `text-muted` cannot drift from gui/src/styles/tokens.ts.
 * See docs/design/tokens.md.
 *
 * `background` and `foreground` keep their old names because the existing
 * stylesheet uses them; they now resolve to the token values.
 */
const config: Config = {
  content: [
    "./src/pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/components/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        background: "var(--background)",
        foreground: "var(--foreground)",

        bg: "var(--bg)",
        surface: "var(--surface)",
        "surface-raised": "var(--surface-raised)",
        ink: "var(--ink)",
        muted: "var(--muted)",
        line: "var(--line)",
        "line-strong": "var(--line-strong)",

        // Brand: logo, primary CTA, progress, current step only.
        "brand-acid": "var(--brand-acid)",
        "on-acid": "var(--on-acid)",
        "brand-ink": "var(--brand-ink)",

        // Status: a different hue from the brand on purpose.
        "success-bg": "var(--success-bg)",
        "success-ink": "var(--success-ink)",
        "success-border": "var(--success-border)",
        "info-bg": "var(--info-bg)",
        "info-ink": "var(--info-ink)",
        "warn-ink": "var(--warn-ink)",
        "warn-bg": "var(--warn-bg)",

        // Diff rows are their own family: an added line and a passed check
        // are not the same message.
        "diff-added-bg": "var(--diff-added-bg)",
        "diff-added-ink": "var(--diff-added-ink)",
        "diff-removed-bg": "var(--diff-removed-bg)",
        "diff-removed-ink": "var(--diff-removed-ink)",
        "diff-meta-ink": "var(--diff-meta-ink)",

        outline: "var(--outline)",

        // Loud tile fills. Same value in both schemes, unlike the status inks
        // above -- see the comment on --tile-info in styles/tokens.css.
        "tile-info": "var(--tile-info)",
        "tile-success": "var(--tile-success)",
        "tile-warn": "var(--tile-warn)",
      },
      borderRadius: {
        brand: "var(--radius-brand)",
        chip: "var(--radius-chip)",
        control: "var(--radius-control)",
        panel: "var(--radius-panel)",
      },
      transitionTimingFunction: {
        standard: "var(--ease-standard)",
      },
      transitionDuration: {
        enter: "var(--duration-enter)",
        standard: "var(--duration-standard)",
        exit: "var(--duration-exit)",
      },
    },
  },
  plugins: [],
};
export default config;
