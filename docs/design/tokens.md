# Design tokens

The single source of truth for colour, type, radius and motion across the three
surfaces. Column widths and the shell's geometry are [layout.md](layout.md) — this file
is about what things look like, not about where they sit. Implementation mirrors:

| Surface | Lives in | Consumed by |
|---|---|---|
| GUI (Tauri + React) | `gui/src/styles/tokens.css` | CSS variables, read by `base.css` and every component's `.module.css` |
| Web (Next.js) | `web/src/styles/tokens.css` | CSS variables + `web/tailwind.config.ts` |
| TUI (Rust) | `crates/firment-tui/src/theme.rs` | ratatui styles, with a colour-degradation chain |

The GUI's stylesheet is **three layers, and the boundary between them is asserted
rather than agreed**: `tokens.css` declares values and draws nothing (its only
selectors are the two scheme blocks and one scheme-independent `:root`), `base.css`
is the only place element and global selectors may appear, and everything else is a
`<Component>.module.css`. `gui/src/ui/__tests__/conventions.test.ts` enforces that
over a directory list, and `styles/__tests__/no-literal-tokens.test.ts` rejects a
bare hex or px in any `.ts`/`.tsx` under `gui/src`. Both exist because "there is one
place a design value is written down" decays one inline literal at a time.

The token names below are the custom properties as they are written, so anything in a
table can be grepped in `gui/src/styles/tokens.css`. A ratio pair in a table is written
`dark / light`, matching the column order, and it ends by naming the token it was measured
on — `1.61 / 1.48 on --surface`, the ground in backticks in the row itself — because
`gui/src/styles/__tests__/tokens.test.ts` scrapes these tables and recomputes every quoted
number out of the stylesheet. It compares the hexes in the table against the stylesheet as
well, and the key count this document announces below. What that gate does **not** cover is
a ratio written as prose instead of as a tabulated pair: the scraper never sees it, so an
untabulated number is a claim, not a measurement.

## Choosing a scheme

`ui.theme` in config.toml — `auto` (default) / `light` / `dark` — resolved by
`gui/src/lib/theme.ts`. GUI and web read the same setting so the two cannot drift
into different greys.

**`auto` follows the OS and nothing else.** It is the default, so on a dark
machine `auto` resolves to dark; a user who wants to *see* the light scheme must
be able to pin `light`. Any "smart" fallback would make light unreachable on
exactly the machines the author works on.

The TUI ignores the setting. Its colours come from the terminal's own palette, so
light-versus-dark there is the user's terminal theme, and overriding that from a
config file would fight the thing that already knows the answer.

## Colour

### The ramp is olive, and it is the logo's

Every neutral sits at about **hue 88** — yellow-green. That is not a preference; it
is what this project's own landing page does, measured: `#414a35` is 86°, `#647257`
91°, `#a0b28a` 87°, and its one bright colour `#b4f779` is 92°.

The app used to be perfectly neutral (`--line: #3a3a3a`, `--muted: #b4b4b4`), which
is the plainest reason the interface and its own logo read as two different products.
The two schemes now each match a logo that already existed:

| Scheme | `--brand-acid` | Source |
|---|---|---|
| dark | `#b4f779` | `gui/public/icons/logo-*.png` |
| light | `#4d7c0f` | `gui/public/icons/logo-w-*.png` |

Both scheme blocks carry **the same 48 keys**, and the count is asserted — by
`tokens-css.test.ts` against the stylesheet, and by `tokens.test.ts` against this number —
because adding a key to one scheme and forgetting the other is the bug that test exists to
catch.

### Grounds and surfaces

Four steps, each tinted rather than merely lighter.

| Token | dark | light | Use |
|---|---|---|---|
| `--bg` | `#090b09` | `#fbfcf8` | the window |
| `--surface` | `#0d130a` | `#f5f7f0` | a panel: rail, inspector, status bar |
| `--surface-raised` | `#10160c` | `#edf0e6` | a card, a row, the base of a control |
| `--code-bg` | `#0a0e07` | `#e9ede1` | a code block nested inside a card |

`--code-bg` goes **down**, not further up: the nesting runs ground < panel < card <
code, so a block inside a card is the darkest surface in it. That is the order the
landing page builds its showcase in.

The light scheme's `--surface-raised` is a real step from `--surface` (1.07:1) and a
test pins that they differ, which is the trap that once made a selected row invisible
in light only.

### Edges — four weights, and one of them carries a selection

| Token | dark | light | On | Ratio |
|---|---|---|---|---|
| `--line` | `#242c1e` | `#e2e6da` | inside a card | 1.30 / 1.17 on `--surface` |
| `--border` | `#2f3c25` | `#c8d0b9` | a card's own edge | 1.61 / 1.48 on `--surface` |
| `--border-strong` | `#3a472e` | `#bcc4ab` | a panel, a window, a field | 1.99 / 1.75 on `--bg` |
| `--border-active` | `#88b366` | `#6d8c4a` | a selected or running thing | 7.79 / 3.54 on `--surface` |

`--border-strong` is the renamed `--outline`: same role, a name that says which tier
it is. A test asserts the ladder is strictly increasing in both schemes, because two
of them collapsing to one value is what makes nesting unreadable and no reviewer
would see it in a diff.

`--border-active` is not decoration; it is **the** selection signal, because the fill
cannot be one (see Interaction).

### Text — four levels

| Token | dark | light | Ratio | Use |
|---|---|---|---|---|
| `--ink` | `#eef1e8` | `#1a1e15` | 16.49 / 15.68 on `--surface` | headings, prose, tool names |
| `--ink-soft` | `#d3d9c9` | `#3a4230` | 13.04 / 9.71 on `--surface` | secondary prose |
| `--muted` | `#a9b09f` | `#5c6353` | 8.43 / 5.78 on `--surface` | labels, counts, structure |
| `--dim` | `#828f70` | `#656c58` | 5.48 / 5.06 on `--surface` | paths, times, counters, an empty state's aside |

There were two (`ink` and `muted`), which meant a path and a heading could only be
told apart by size. `--dim` is the rung that binds: it is the quietest, and it still
has to clear AA on a card, on the code block inside it, and on a selection.

**`--muted` is olive now, and that reverses a documented rule.** The old comment read
`Never olive: reads as disabled.` It was true while the rest of the ramp was neutral —
an olive grey among neutral greys is the only one that looks switched off. With every
neutral olive there is nothing left for it to stand out against, so the colour can no
longer say "disabled" and the second signal is opacity: `--disabled-opacity: 0.55`,
applied by every `:disabled` rule, listed and checked in `conventions.test.ts`.

### Brand — two values again, and they are not interchangeable

| Token | dark | light | Use |
|---|---|---|---|
| `--brand-acid` | `#b4f779` | `#4d7c0f` | the fill; equal to the logo in each scheme |
| `--on-acid` | `#15200d` | `#ffffff` | the label on that fill (13.28 / 4.99 on `--brand-acid`) |
| `--brand-ink` | `#b4f779` | `#476f0e` | brand-coloured **text** (14.82 / 5.48 on `--surface`) |

They are the same colour only in dark. In light, the logo's own green as text on a
card ground is 4.33:1 — under AA — so text takes a darker value. This is the same
trap the old palette had and did not notice: `#107d98` on `--surface-raised` was
4.18:1.

`--brand-ink` is also what the `brand` icon tone resolves to, which is why that tone
can exist at all now (see the note in `gui/src/ui/Icon.tsx`; the old palette could not
afford it, and the rule against it was a measurement rather than a taste).

### Status, diff and steps

Success keeps a **cooler hue than the brand on purpose** — 151° against the lime's
92°, which is 59° of separation. That is *more* than the cyan brand it replaces had
against its own success green (190° vs 144° = 46°), so going lime does not blur
"brand" and "passed".

| Family | dark bg / ink | light bg / ink |
|---|---|---|
| success | `#16240f` / `#5fd39b` | `#dff0d8` / `#1f6b41` |
| info | `#0e1c2a` / `#9fd0f5` | `#e6f0f8` / `#14456b` |
| warn | `#2a2008` / `#ffd98a` | `#fdf3d2` / `#6b4a06` |
| diff added | `#16230d` / `#c9e8a2` | `#e4f3d5` / `#24400f` |
| diff removed | `#2e1216` / `#ffbcc2` | `#fbe0e3` / `#6d1a20` |

`--diff-meta-ink` is `--muted`: a hunk header is structure, not content.

The three step states are derived rather than invented — `--step-done-*` is
`--diff-added-*` and `--step-failed-*` is `--diff-removed-*`, the same message at two
sizes, and a test pins that they stay shared. `--step-rule` is a shape rather than
text, so it only owes 3:1; in light it is `#6d8c4a` (3.71:1 on `--bg`) because the
acid would be 4.85:1 as a *fill* and a 2px rule is not a fill.

### Interaction

| Token | dark | light | Note |
|---|---|---|---|
| `--hover` | `#ffffff24` | `#e6ebdc` | a wash, not a grey |
| `--wash-resting` | `#ffffff14` | `transparent` | a control with nothing else to sit on |
| `--field-bg` | `#ffffff0f` | `--surface` | a translucent white, never an opaque grey |
| `--selection` | `#1a2613` | `#dff0c2` | an active surface |
| `--on-selection` | `#eef1e8` | `#1a1e15` | text on it (13.82 / 14.01 on `--selection`) |
| `--focus-ring` | `#b4f779` | `#476f0e` | ≥3:1 as a shape, in both |

**`--selection` is a surface and the fill is auxiliary.** Its value is the darkest
fill that still holds all four text levels at AA on top of it. A more visible fill
(1.43:1 against a row's ground instead of 1.17) drops `--dim` to 3.74 and fails. What
marks a selection is `--border-active`. Note also that `--selection` is not only a
selected row: the running tool card, the verdict band and a pressed icon button all
use it, so "what gets painted on it" is a longer list than "what a row contains" —
check all of them before changing it. Every one of those pairs is in the test table.

`--hover` and `--wash-resting` are translucencies rather than greys because an opaque
grey on a near-black ground is mud: no light passes through it. Light has no need of
the wash, where the ground and the hairline already give a control an edge.

## Button weight — four tiers

Weight is carried by **colour and fill, never by size**: every tier is the same height
and sits level on a row.

| Tier | Treatment |
|---|---|
| primary | `--brand-acid` fill, `--on-acid` label |
| secondary | `--hover` fill, `--border-strong` hairline |
| ghost | no chrome, `--muted` label, `--ink` when it is an icon-only button |
| danger | `--step-failed-bg` fill, `--step-failed-ink` label |

All four are the layer's own `Button`, so hover, active, disabled and loading come
from `Button.module.css` and nowhere else.

**A fill and its ink are never chosen per call site**, and the pair is asserted in
tokens.test.ts. A fill and an ink taken from different tokens is what once put
near-white text on a filled user bubble — about 1.3:1, in the dark scheme only.

## Type

Two tracking tokens for two decisions: `--tracking-label` (0.16em, positive) for
uppercase Latin micro-labels, and `--tracking-display` (−0.02em, negative) for the
wordmark. One value used for both would have been a coincidence wearing a system's
clothes. `--tracking-title` (−0.4px) is the heading rung; `--tracking-cjk` (0.02em)
exists because **0.16em is designed for uppercase Latin** and reads as a line of
separated characters under CJK.

Uppercase is a rule rather than a typed string: `Chip` takes `upper`, which sets
`text-transform` and the label tracking in CSS. It is scoped to labels — a small chip
is just as likely to hold a path, and `PA5` is a token before it is a word.

| Surface | Token | Stack |
|---|---|---|
| GUI | `--ff-sans` | `'DM Sans Variable', 'MiSans', 'HarmonyOS Sans SC', 'Microsoft YaHei UI', 'PingFang SC', 'Noto Sans CJK SC', system-ui, 'Segoe UI', sans-serif` |
| GUI | `--ff-mono` | `'IBM Plex Mono', 'Sarasa Mono SC', 'Microsoft YaHei UI', 'Cascadia Mono', Consolas, monospace` |
| Web | `--font-sans-stack` | `'Inter', 'Noto Sans SC', system-ui, -apple-system, 'Segoe UI', sans-serif` |
| Web | `--font-mono-stack` | `'JetBrains Mono', 'Noto Sans SC', 'Cascadia Code', Consolas, monospace` |

**Prose and code are separate, and the line is drawn by voice rather than by
element.** Text a person reads — a heading, a paragraph, a button's label — is the
sans face. Everything the machine reports is the mono face: a tool name, a path, a
duration, a counter, a tab label, a step's name. The rule that used to live here,
"setting UI text in a monospace makes every label shout", was written against a tree
where prose *and* chrome were monospace, which is a page of equal-width text.
Monospace for the machine's half is the opposite of shouting — it is how a report is
told apart from a sentence. `conventions.test.ts` holds the list of machine-voice
files so the split cannot rot quietly.

**The GUI names a CJK face and bundles none.** `system-ui` alone is what produced the
complaint that started this: Latin rendered in the bundled face and Chinese in
whatever the platform picked, so one line of mixed text had two faces in it. Naming
`'Microsoft YaHei UI'` (which hints better at small sizes than plain `YaHei`),
`'PingFang SC'` and `'Noto Sans CJK SC'` costs nothing and fixes the mixture; bundling
one would cost megabytes for glyphs the system already has.

**The first family in each stack is the name the package declares.** The variable
packages ship `'<Name> Variable'` and the static ones the plain name — `--ff-sans`
said `'DM Sans'` for a while, which matched nothing, so the app fell back to the
system face while the token claimed otherwise. That is the Inter failure again, and
`tokens-css.test.ts` now reads the declared name out of the package itself.

Fonts arrive as `@fontsource-variable/dm-sans/wght.css` plus `latin-400.css`,
`latin-500.css` and `latin-600.css` of `@fontsource/ibm-plex-mono` — IBM Plex Mono has
no variable release, so it is imported one weight at a time, and only `latin`. The
three that are imported are exactly the three the ramp uses for mono (`--fw-book`,
`--fw-medium` behind `--fw-mono`, `--fw-label`); a weight nobody imported is not a
fallback but a *synthesised* one, the browser smearing the 400 outlines. Anything
outside `latin`, CJK included, falls through to the named faces in the stacks above.

`--lh-body` is 1.85, measured off the landing page's own prose (14/25.9, 11/20.35,
12/22.8). It is the one value in this palette that is inherited rather than derived;
1.75 is the usual Chinese answer if it ever reads too loose.

### The weight ramp

Five named weights, and only three of them are imported for the mono face:

| Token | Value | Where it is the whole decision |
|---|---|---|
| `--fw-book` | 400 | prose, and a step that has not run yet |
| `--fw-medium` | 500 | the wordmark, a tool's name — the sans step the design asked for and the ramp did not have |
| `--fw-mono` | `var(--fw-medium)` | every rule that sets the mono face; one number, named for the face it belongs to |
| `--fw-label` | 600 | a label above its surroundings |
| `--fw-strong` / `--fw-hero` | 700 / 800 | sans only |

`main.tsx` imports `latin-400/500/600` of IBM Plex Mono and nothing else, because it
has no variable release. A mono rule asking for 700 therefore does not get a heavier
cut — it gets the browser smearing the 600 outlines. `tokens-css.test.ts` reads the
imported set out of `main.tsx` and fails any rule that sets the mono face at a weight
outside it; the check exists because one `--fw-strong` on a stat number sat in the
tree through a whole restyle.

### The size scale

| Token | Value | Role |
|---|---|---|
| `--fs-micro` | 10px | uppercase micro-labels, counters |
| `--fs-meta` | 11px | chips, tags, tab labels, timestamps, mono paths |
| `--fs-minor` | 12px | tool args, session meta, the status bar |
| `--fs-body` | 13px | **default**: prose, inputs, log lines, card titles |
| `--fs-ui` | 14px | control labels, the active inspector tab |
| `--fs-read` | 15px | text you read: the transcript |
| `--fs-title` | 17px | a card's or a panel's heading |
| `--fs-display` | 28px | the wordmark, and nothing else |

`--fs-title` is the rung that was missing: the scale used to run 10–15 and jump
straight to 28, so a card title had nowhere to go and the tool card shipped its title
at `--fs-minor` — smaller than the body text beside it.

### Two numbers that are not steps of the ramp

`--gap-card` (10px) is the space between two cards. The spacing ramp's 8px reads as
"attached to the row above" and its 16px — which is what the transcript uses between
messages — reads as "two unrelated things"; consecutive cards are one call's parts, so
they need a number of their own, and naming it is what keeps three card stacks from
each spelling it slightly differently.

`--h-strip` (42px) is a tab strip's height, and `--h-bar` (46px) the title bar's.
Neither is `--h-row` (40): a row carries a text baseline and has to line up with
sidebar rows, menu items and status items, where a strip carries a label and the rule
under it. The two used to share the number, which is why the title bar's monospace
path was clipped and the strip was taller than its own tabs. `conventions.test.ts`
pins that the inspector's collapse toggle derives from the SAME token the strip does —
it used to spell `--h-row + 1px` beside the strip's `--h-row`, correct by coincidence,
and a coincidence is invisible in a diff.

## Radius

| Token | Value | Use |
|---|---|---|
| `--r-chip` | `3` | badges, chips, counters, the phase pill |
| `--r-inset` | `4` | a code block nested inside a card |
| `--r-btn` | `5` | buttons, selected rows, icon squares |
| `--r-card` | `6` | a card's own corner |
| `--r-control` | `6` | inputs and other controls |
| `--r-panel` | `8` | panels, modals — and the logo tile, the other shape that shares this corner |
| `--r-round` | `999` | pills, dots, avatars |

Not one value everywhere: a small chip needs to stay crisp, a large panel needs
softness. `0` everywhere reads as unfinished rather than deliberate; `12` on a dense
panel reads as loose, and `12` on a control starts to look like a pill.

`--r-card` and `--r-control` hold the same number and are still two names, because a
card and a control are different objects that happen to share a corner — measured off
the landing page they are 6 and 5, with the tags at 3 and the windows at 8. Keeping
them apart means a change to one cannot drag the other.

`no-literal-tokens.test.ts` enforces the token layer, not just the radius: no literal
corner radius, no literal hex colour and no hand-written font stack anywhere in the
TypeScript under `gui/src`. What it does **not** read is the stylesheets — it globs
`.ts`/`.tsx` only. A bare `px` in a `.module.css` is legal by design, because a
component's own padding is not a design token; a colour there is expected to be a
`var(--…)` reference. That half is convention rather than a gate.

## Motion

- Enter `160ms`, standard `200ms`, exit `120ms`
- Curve `cubic-bezier(.2,.8,.2,1)`
- Stagger `40ms` on grouped elements
- Everything respects `prefers-reduced-motion`
- **TUI caps at 15fps**, not 60: Windows Console, SSH and tmux repaint badly at
  high rates, and `firm` over SSH to a dev board is a normal way to work. The
  25ms input tick is not the frame rate — animation repaints are throttled
  separately, so a repaint caused by real output is never delayed.
- The TUI disables motion entirely under `SSH_CONNECTION`/`SSH_TTY`,
  `TERM=dumb`, or a non-TTY stdout, and takes `--no-anim` for a capable
  terminal where the spinner still is not worth the repaints. See
  `crates/firment-tui/src/motion.rs`.

These numbers were left alone by the restyle, and that was a decision rather than an
omission: the landing page's own motion is three keyframes and a flat 200ms (a 3–4px
rise plus a fade on entry, a 2s soft blink for a caret), which is *coarser* than this
ramp, not finer. There was nothing here to adopt.

## State semantics

Three states must be visually distinct, and **red is reserved for real errors**:

| State | Treatment |
|---|---|
| `StateUnknown` | Neutral `○`, muted ink. **Never rendered as off** — unknown is not failed |
| `StateOffline` | Muted, with the last-seen time |
| `StateError` | Only after a user action actually failed |

The rule for any new panel: **unconfigured → hidden entirely; configured but empty
→ neutral copy; only a real failure → red.**

A disabled control is a fourth thing, and it is not a state: it is dimmed by
`--disabled-opacity` (0.55) in addition to whatever colour change the tier makes,
because with an all-olive ramp colour alone is no longer a signal.
