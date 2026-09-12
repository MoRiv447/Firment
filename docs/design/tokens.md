# Design tokens

The single source of truth for colour, type, radius and motion across the three
surfaces. Implementation mirrors:

| Surface | Lives in | Consumed by |
|---|---|---|
| GUI (Tauri + React) | `gui/src/styles/tokens.ts` | `ConfigProvider` via `antdTheme(mode)` |
| Web (Next.js) | `web/src/styles/tokens.css` | CSS variables + Tailwind |
| TUI (Rust) | `crates/firment-tui/src/theme.rs` | ratatui styles, with a colour-degradation chain |

The GUI has **no stylesheet** — `gui/src` contains zero `.css` files and every
colour is an inline hex or an antd theme token. That is why the GUI's token layer
is data mapped into `ConfigProvider` rather than a CSS-variable file: CSS
variables would be invisible to antd's own components.

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

Neutral grounds and neutral text, with the brand green in exactly four places:
the logo, the primary CTA, progress, and the current step. An interface tinted
green makes the diff's own red and green harder to read, and Firment's screens
are mostly diff.

**A contrast ratio is meaningless without its ground.** `#F7F7F5` and `#FFFFFF`
look like the same white, but they are 0.3 apart on `ink` and `muted`. Every
ratio below names the ground it was measured against, and
`gui/src/styles/__tests__/tokens.test.ts` asserts the light ones.

### Grounds and text

| Token | Dark | Light | Light ratio |
|---|---|---|---|
| `bg` | `#0F0F12` | `#F7F7F5` | — |
| `surface` | `#18181B` | `#FFFFFF` | 1.07:1 vs `bg` — separation comes from the hairline, not the fill |
| `surfaceRaised` | `#1F1F23` | `#FFFFFF` | distinguished by border and shadow |
| `ink` | `#E4E4E7` | `#18181B` | 17.72:1 on `surface`, 16.52:1 on `bg` |
| `muted` | `#A1A1AA` | `#71717A` | 4.83:1 on `surface`, **4.51:1 on `bg`** (the tighter of the two) |
| `line` | `#2A2A2F` | `#E4E4E7` | 1.18:1 — a hairline, not the 3:1 non-text threshold |
| `lineStrong` | `#3F3F46` | `#D4D4D8` | 1.48:1 on `surface`; secondary button outlines |

Dark reads: `ink` 15.08:1 on `bg`, `muted` 7.47:1.

`muted` is a **neutral grey, never olive** — olive reads as disabled. On the light
ground `muted` is already at 4.51:1, which is the AA floor: nothing may be placed
under it that darkens the ground without re-measuring.

### Brand vs status green — two different colours on purpose

| Token | Dark | Light | Light ratio |
|---|---|---|---|
| `brandAcid` | `#B4F779` | `#B4F779` | **1.27:1** — fill only |
| `onAcid` | `#15200D` | `#15200D` | 13.28:1 **on `brandAcid`** |
| `brandInk` | `#3B6D11` | `#3B6D11` | 6.21:1 on `surface`, 5.79:1 on `bg` |

`brandAcid` is 85° (acid lime); the success green is 145° (true green). They are
60° apart, so they read as different things: the brand colour is *identity*, the
status colour is *feedback*. Using one for both made "this is Firment" and "this
passed" look identical.

**`brandAcid` is ~1.8:1 on a light ground.** It is a highlighter, never a text or
icon colour. Anything readable that is green-on-light uses `brandInk`.

### Status

| Token | Dark | Light | Light ratio |
|---|---|---|---|
| `successBg` | `#14532D` | `#DCFCE7` | — |
| `successInk` | `#86EFAC` | `#15803D` | 5.02:1 on `surface`, 4.57:1 on `successBg` |
| `successBorder` | `#166534` | `#BBF7D0` | — |
| `infoBg` | `#1A1E26` | `#E0F2FE` | — |
| `infoInk` | `#7DD3FC` | `#0369A1` | 5.93:1 on `surface`, 5.17:1 on `infoBg` |
| `warnBg` | `#3F2E06` | `#FEF3C7` | — |
| `warnInk` | `#EAB308` | `#B45309` | 5.02:1 on `surface`, 4.51:1 on `warnBg` |

`antdTheme()` maps `colorSuccess` to `successInk` in **both** schemes. Feeding the
light scheme `brandInk` — as the first draft did — would paint "this passed" in
the brand green, which is the exact confusion the 85°/145° split exists to
prevent.

**The dark status inks do not transfer to light.** `#7DD3FC` and `#EAB308` are
~2:1 on a white ground; both need the light value above. Likewise `#16A34A` is
not usable as *text* on light — the readable green there is `#15803D`.

### Diff

| Token | Dark | Light | Light ratio |
|---|---|---|---|
| `diffAddedBg` / `diffAddedInk` | `#14311C` / `#86EFAC` | `#DCFCE7` / `#15803D` | 4.57:1 |
| `diffRemovedBg` / `diffRemovedInk` | `#3B1218` / `#FDA4AF` | `#FEE2E2` / `#9F1239` | 6.56:1 |
| `diffMetaInk` | `#A1A1AA` | `#71717A` | 4.51:1 (hunk headers, context) |

Diff colours are their own family rather than reusing the status colours: an added
line and a passed check are not the same message.

A tool card also shows the counts — `+12 -3` — in this family, right-aligned in the
header. They are counted the same way the TUI counts them
(`crates/firment-tui/src/view.rs`): over the diff body only, skipping the
"Edited `<path>`" line the body drops.

### Interaction

| Token | Dark | Light | Notes |
|---|---|---|---|
| `hover` | `rgba(255,255,255,0.08)` | `#F6FEEF` | row and menu hover wash |
| `focusRing` | `#B4F779` | `#3B6D11` | keyboard focus |
| `outline` | `#3F3F46` | `#D4D4D8` | the border on a card, chip or control |
| `shadowSm` | `0 1px 2px rgba(0,0,0,.32)` | `0 1px 2px rgba(16,24,40,.06)` | raised rows, the selected session |
| `shadowMd` | `0 4px 12px rgba(0,0,0,.36)` | `0 4px 12px rgba(16,24,40,.08)` | cards that need to lift |
| `shadowLg` | `0 12px 32px rgba(0,0,0,.44)` | `0 12px 32px rgba(16,24,40,.12)` | popovers, dropdowns, modals |

The light hover is an **acid tint, not a grey**. `muted` is 4.51:1 on `bg` and
that ground is already the AA floor, so any grey dark enough to read as a hover
pulls a muted label under it (4.40:1 at `#F4F4F5`, 4.47:1 at `#F6F6F7`). The tint
is the only candidate that holds — 4.68:1 for `muted`, 17.16:1 for `ink` — and it
reads as a weaker sibling of the solid-acid selection rather than competing with
it.

The focus ring is **not** the acid in the light scheme: acid on a light ground is
1.27:1 and a keyboard user cannot see it. The dark scheme can afford the acid ring
(15.06:1); the light scheme uses `brandInk`.

**`outline` is a grey in both schemes, and the shadow is the second separator.**
It used to be `#000000` in both, carrying the neo-brutalist frame: 2px and 3px
borders with a hard offset block (`3px 3px 0`) behind them. That layer is gone.
Two surfaces are now told apart by a hairline first and a shadow second, so
`shadowSm` sits under raised rows and `shadowMd`/`shadowLg` are reserved for
things that genuinely overlap live content. The name `outline` survived the
change because ~60 call sites read it and they all mean the same thing by it —
"the border on this thing" — but it carries no brand meaning any more.

The light shadows are wider and much lower-alpha than the dark ones on purpose:
the same black that reads as depth on `#0F0F12` reads as dirt on `#F7F7F5`.

### Steps

The build / flash / monitor progress row. Three visible states plus an unknown
one, and the row is **never interactive**: no hover, no pointer, no click target.
A progress row that looks pressable becomes a control that does nothing.

| State | Dark | Light | Light ratio on `bg` |
|---|---|---|---|
| done | `#14532D` fill, `#86EFAC` ink | `#EAF3DE` fill, `#3F6212` ink | 6.19:1 |
| failed | `#3B1218` fill, `#FDA4AF` ink | `#FEE2E2` fill, `#9F1239` ink | 6.56:1 |
| current | no fill, `#E4E4E7` ink, 2px `#B4F779` rule | no fill, `#18181B` ink, 2px `#B4F779` rule | 16.52:1 |
| pending | transparent, `#A1A1AA` ink | transparent, `#6B7280` ink | 4.51:1 |
| unknown | transparent, muted ink, `○` | same | — |

Two rules matter more than the values:

- **A pending step stays legible.** It reads as "not yet", not as "unavailable",
  so it keeps the pending ink rather than being greyed to disabled.
- **Unknown is not failure.** It renders `○` with muted ink and never `✗`; red is
  reserved for a real error, and a step nobody has measured yet is not one.

`failed` is the one state that may be red, and it borrows the removed-diff pair
rather than introducing a fourth red: "this broke" and "this was taken out" are
the same message at different sizes. Only `done` and `failed` are filled — they
are the only two states whose outcome is already known.

The row is **derived from the turn's tool calls** (`gui/src/lib/steps.ts`), not
tracked as its own state, so it cannot disagree with the tool cards next to it. A
turn that never built anything shows no row at all, and a later attempt
supersedes an earlier one: a build that failed and then passed reads as `done`.

## The slant is gone

This section used to be titled "Slant — the one signature that cannot be
substituted", and it described a `clip-path` cut on the primary CTA: a 12px
horizontal run, a 5px optical correction on the left, and two absolutely
positioned clipped layers so the diagonal had an edge a `border` cannot draw on
a clipped element.

It was removed with the rest of the neo-brutalist layer. The cut is a loud
geometric gesture, and it only made sense as one element of a frame that no
longer exists: once the black outlines and the hard offset shadows went, a
single slanted button read as an accident rather than as a signature.

The logo still carries the skew (three slanted bars) — that is the brand's
structural mark and it is **drawn into the artwork**, not into a control. Nothing
in the interface reproduces it.

Removing it also removed a real defect. The primary tier's dark edge was a layer
filled with `color.outline`, which was pure black while the visual layer was
brutalist. When `outline` became an ordinary border grey, that layer turned the
primary button into an acid fill inside a grey ring — a fill that looks outlined
by mistake. `ActionButton` now delegates every tier to antd (`primary` /
`default` / `text`), so there is no layer left to get this wrong, and
`designSystem.test.tsx` asserts that a primary button carries no inline border,
shadow or fill.

What survived is `space.controlGap` (8px). The gap between controls on a row was
never about the slant.

### Button weight — three tiers

Weight is carried by **colour and fill, never by size**: all three tiers render
at antd's control height and sit level on a row.

| Tier | Treatment |
|---|---|
| primary | `brandAcid` fill, `onAcid` label. One per screen |
| secondary | `surface` fill, `lineStrong` hairline |
| tertiary | no chrome: a `muted` label and a chevron, going to `ink` on hover |

The fill and the label are not set per call site: `antdTheme` maps `colorPrimary`
to `brandAcid` and `Button.primaryColor` to `onAcid`, so a primary button cannot
be assembled from a fill and an ink measured against different grounds. That
mistake is what put near-white text on the acid user bubble (about 1.3:1, in the
dark scheme only), and it is now asserted in `userBubble.test.tsx` for both
schemes.

## Type

| Token | Stack |
|---|---|
| `sans` | `'Inter', 'Noto Sans SC', system-ui, -apple-system, 'Segoe UI', sans-serif` |
| `mono` | `'JetBrains Mono', 'Noto Sans SC', 'Cascadia Code', Consolas, monospace` |

UI text and code are separate. The GUI previously set the whole interface in a
monospace, which made every label shout.

`'Noto Sans SC'` must be listed explicitly or Chinese falls back to Microsoft
YaHei and sits visibly wrong next to the Latin text.

## Radius

| Token | Value | Use |
|---|---|---|
| `tile` | `8` | Logo and icon tiles, and the cards built on the same shape |
| `chip` | `4` | Badges, chips, tooltips |
| `control` | `6` | Inputs and buttons |
| `panel` | `12` | Cards, panels, modals |

Not one value everywhere: a small chip needs to stay crisp, a large panel needs
softness. `0` everywhere reads as unfinished rather than deliberate; `12` on a
control starts to look like a pill.

**These were `0 / 2 / 4 / 8` with `brand` at 0.** The name is gone along with the
value it stood for: `brand` (0) meant "the brand anchor stays hard-edged", which
was the neo-brutalist frame speaking. Once the black outline around a card went
away, a 0 on that card read as an unfinished box rather than a deliberate one, so
the tiles round like everything else and the tier is named for what it is.

Applied by element, not by habit: badges and chips take `chip`, alerts, inputs,
buttons and selectable rows take `control`, cards and panels take `panel`, and
`tile` belongs to the logo and icon tiles.

`gui/src/styles/__tests__/no-literal-tokens.test.ts` enforces the whole token
layer, not just the radius: no literal corner radius, no literal hex colour and
no hand-written font stack anywhere under `gui/src` outside `tokens.ts`. The
claim that there is one place a design value is written down decays one inline
literal at a time, so it is asserted rather than trusted.

Note what that test does **not** catch: an antd preset colour name (`color="blue"`,
`'purple'` in a ternary) is neither a hex literal nor a radius, so it sails
through. It is still a way to write a colour down outside this file, and it is
how the header ended up with five hues — including a purple that appears nowhere
in this document.

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

## State semantics

Three states must be visually distinct, and **red is reserved for real errors**:

| State | Treatment |
|---|---|
| `StateUnknown` | Neutral `○`, muted ink. **Never rendered as off** — unknown is not failed |
| `StateOffline` | Muted, with the last-seen time |
| `StateError` | Only after a user action actually failed |

The rule for any new panel: **unconfigured → hidden entirely; configured but empty
→ neutral copy; only a real failure → red.**
