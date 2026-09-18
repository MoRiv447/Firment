# Design tokens

The single source of truth for colour, type, radius and motion across the three
surfaces. Implementation mirrors:

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
table can be grepped in `gui/src/styles/tokens.css`. The web file is named after the
GUI's so the two can be read side by side — except the font stacks, which differ by
design (see Type).

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

### The palette is Radix's

Every value below comes from [`@radix-ui/colors`](https://www.radix-ui.com/colors)
— the same scales Supabase builds on. Each hue has 12 steps with a documented role,
and the dark variant of each step is designed rather than derived, which is why the
two schemes are genuinely two decisions instead of one inverted.

| Step | Role | Our token |
|---|---|---|
| 1 | app background | `--bg` |
| 2 | subtle background | `--surface` |
| 3 | UI element background | `--surface-raised`, `*-bg` for a badge |
| 4 | hovered element | `--hover`, a diff line |
| 5 | **active / selected** | `--selection` |
| 6 | subtle borders | `--line` |
| 7 | control borders | `--outline`, `--success-border` |
| 8 | hovered border, focus ring | `--focus-ring`, the dark brand fill |
| 9 | solid fill (highest chroma) | `--brand-acid` in light |
| 10 | hovered solid | button hover |
| 11 | low-contrast text | `--muted`, `--brand-ink` |
| 12 | high-contrast text | `--ink`, the state inks |

Three consequences worth knowing before reading the rest of this file:

* **A state ink sits on step 12, not 11.** Radix guarantees step 11 on step *2* of
  the same scale; our state inks sit on steps 3 and 4, where 11 measured below the
  4.5 floor (success 4.21, diff-added 3.93).
* **The dark brand fill is step 8, and its label is white.** Step 9 is the scale's
  brightest and glows as a large fill on a near-black ground; step 8 with white
  measures 4.57:1. Nothing clears 4.5 on step 9 — white is 3.00 there.
* **Only the light scheme casts shadows.** In dark the whole ladder is `none`, and
  separation is the hairline plus the surface step: a black shadow on a near-black
  ground is invisible and still costs a composited layer.

Values are lifted out of the package by a script rather than retyped — the scales
live under `.dark`/`.light` and this app switches on `:root[data-scheme]`, so they
cannot be imported as they are.

Neutral grounds and neutral text, with the brand green in exactly four places:
the logo, the primary CTA, progress, and the current step. An interface tinted
green makes the diff's own red and green harder to read, and Firment's screens
are mostly diff.

**A contrast ratio is meaningless without its ground.** `#F7F7F5` and `#FFFFFF`
look like the same white, but they are 0.3 apart on `--ink` and `--muted`. Every
ratio below names the ground it was measured against, and
`gui/src/styles/__tests__/tokens.test.ts` asserts the light ones.

### Grounds and text

| Token | Step | What it is |
|---|---|---|
| `--bg` | gray-1 | the app ground |
| `--surface` | gray-2 | panels and bars. One step above the ground, and the hairline is the *second* separator |
| `surface-raised` | gray-3 | a real step above `--surface` in both schemes. It used to equal `--surface` in light, which is what made a selected row invisible there |
| `--ink` | gray-12 | the text you read |
| `--muted` | gray-11 | the second rank: timestamps, model names, paths |
| `--line` | gray-6 | a hairline and nothing more, not the 3:1 non-text threshold |
| `--outline` | gray-7 | control borders, one step stronger than `--line` |

Ratios are asserted in `gui/src/styles/__tests__/tokens.test.ts` -- the pair list is
there, with the numbers, next to the code that fails when one of them moves.

`--muted` is a **neutral grey, never olive** — olive reads as disabled. Its step is
chosen for headroom rather than for the closest match: the light one measures 5.28 on
`--surface`, 4.92 on `--bg` and 4.55 with the hover wash under it, so it stays above AA on
each of the three grounds it actually lands on. A value sitting exactly on the 4.5
floor would put a muted label under the line the first time the wash changed — and
"4.51:1 on the page background" is not a number anyone should have to reason about
twice.

### Brand vs states — separate scales, not a separation rule

| Token | Dark | Light | What it is |
|---|---|---|---|
| `--brand-acid` | cyan **8** | cyan **11** | the solid fill. The step differs by scheme because a bright fill glows as a large shape on near-black, and a bright one is unreadable as a white label |
| `--on-acid` | white | white | the label. Measured against the fill it sits on, in both schemes |
| `--brand-ink` | cyan **11** | cyan **11** | the brand as text. In the light scheme this is a *readable* cyan, so the same hue can be a fill and a label |
| `--selection` | cyan **5** | cyan **5** | the selected row, on its own step rather than borrowed from the brand |

The brand and the states are separate **scales**, not separate rules. Radix builds
every step of every hue against the same contrast model, so a brand cyan and a
status green coexist by construction -- there is no hue-distance rule to enforce,
and the one this file used to state (85° vs 145°, "60° apart") went with the palette
it was written for.

**The brand is no longer fill-only.** The acid lime was ~1.8:1 on a light ground, so
it could never be a label there; cyan-11 on the page is 4.52:1 and can. That is the
concrete difference the palette change bought, and it is why `--brand-ink` is now the
same hue as the fill rather than a darker green chosen to be legible.

The brand colour is *identity*; the status colours are *feedback*. They are separate
scales of the same system, which is what keeps "this is Firment" and "this passed"
from looking like the same statement.

**The light fill is deep enough to be a label; the dark one is not.** Which is why
`--brand-acid` is cyan-11 in light and cyan-8 in dark, and why the label is white in
the light scheme and white in the dark one for a different reason -- see the table
above. Ratios are asserted in `styles/__tests__/tokens.test.ts` rather than quoted
here, so that a value cannot move without a test failing.

### Status

| Token | Step | What it is |
|---|---|---|
| `--success-bg` / `--success-ink` / `--success-border` | green **3 / 12 / 7** | a positive result |
| `--info-bg` / `--info-ink` | blue **3 / 12** | a neutral fact. Blue rather than cyan, because cyan is the brand |
| `--warn-bg` / `--warn-ink` | amber **3 / 12** | a caution |

Status colours are their own scales -- green, amber, blue, red -- and each is used by
role rather than by hue proximity: `--success-ink` is the green text step, `--warn-ink` the
amber one, and the pairs are asserted in `styles/__tests__/tokens.test.ts` rather than
quoted here.

**A dark status ink does not transfer to light, which is why the ink is a step and
not a value.** An amber bright enough to read on near-black is about 2:1 on white;
the light scheme takes a different step of the same scale. This is what "the dark
scales are designed rather than inverted" means in practice.

### Diff

| Token | Step | What it is |
|---|---|---|
| `--diff-added-bg` / `--diff-added-ink` | green **4 / 12** | an added line |
| `--diff-removed-bg` / `--diff-removed-ink` | red **4 / 12** | a removed line |
| `--diff-meta-ink` | gray **11** | hunk headers and context |

Diff colours are their own family rather than reusing the status colours: an added
line and a passed check are not the same message.

A tool card also shows the counts — `+12 -3` — in this family, right-aligned in the
header. They are counted the same way the TUI counts them
(`crates/firment-tui/src/view.rs`): over the diff body only, skipping the
"Edited `<path>`" line the body drops.

### Interaction

| Token | Dark | Light | Notes |
|---|---|---|---|
| `--hover` | gray-alpha **4** (a translucent white) | gray **4** | the hover and pressed wash. It is a *wash*, not a grey: an opaque step on near-black reads as mud because no light passes through it |
| `--selection` | cyan **5** | cyan **5** | the chosen row -- the step Radix documents for exactly this |
| `--on-selection` | gray **12** | gray **12** | text and icons inside it |
| `--focus-ring` | cyan **8** | cyan **10** | keyboard focus. Step 8 is the documented ring, but cyan-8 on white is 2.32:1 -- under the 3:1 a ring needs -- so the light scheme uses a deeper step |
| `--outline` | gray **7** | gray **7** | the border on a card, chip or control. `--line` is gray **6**, one step quieter, and they are distinct values now rather than one merged in |
| `wash-resting` | a translucent white | `transparent` | an icon-only control on a bar. A value, not a branch: the dark scheme wants something under it, the light one does not |
| `field-bg` | a translucent white | `var(--surface)` | the same reasoning, for field backgrounds |
| `--shadow-sm` | `none` | `0 1px 2px rgba(16,24,40,.06)` | a control that means "press me" |
| `--shadow-md` | `none` | `0 4px 12px rgba(16,24,40,.08)` | transient overlays |
| `--shadow-lg` | `none` | `0 12px 32px rgba(16,24,40,.12)` | what dims the page behind it |

**The selected row has its own step.** It used to be the brand fill, and the pair
that fill needed (`--on-acid` on `--brand-acid`) had to be re-derived per scheme, because
a fill that reads well on near-black is a smear on cream. Radix gives selection a
step of its own, so this is no longer a pair that has to hold together -- it is a
background, with the ordinary ink on top. One rule survives from that arrangement
and is enforced by `styles/__tests__/tokens.test.ts`:

- text and icons inside a selected row read `--on-selection`, **never** `--ink`;
- a chip that lands on a selected row inverts to the pair, because every status
  pair in the table is measured against `--bg`/`--surface` and neither of those is the
  ground under it.

The dark hover is **translucent, and that is the whole point of it.** It was
briefly an opaque grey step after the palette moved, and it read as a flat panel
laid on the row rather than a change in the row: nothing passes through an opaque
wash on a near-black ground. Radix ships alpha scales for exactly this, and the
difference between `#ffffff1b` and `#2a2a2a` is the difference between "lit" and
"painted on".

The focus ring is **step 8 in dark and step 10 in light**. Step 8 is the step Radix
documents for a ring, and cyan-8 on white measures 2.32:1 -- under the 3:1 a ring
needs, which is the one contrast floor that is about a *graphic* rather than text.
The light scheme takes a deeper step and clears it.

**`--outline` is a grey in both schemes, and the shadow is the second separator.**
It used to be `#000000` in both, carrying the neo-brutalist frame: 2px and 3px
borders with a hard offset block (`3px 3px 0`) behind them. That layer is gone.
Two surfaces are now told apart by a hairline first and a shadow second, so
`--shadow-sm` sits under raised rows and `--shadow-md`/`--shadow-lg` are reserved for
things that genuinely overlap live content. The name `--outline` survived the
change because ~60 call sites read it and they all mean the same thing by it —
"the border on this thing" — but it carries no brand meaning any more.

The light shadows are wider and much lower-alpha than the dark ones on purpose:
the same black that reads as depth on `#0F0F12` reads as dirt on `#F7F7F5`.

### Steps

The build / flash / monitor progress row. Three visible states plus an unknown
one, and the row is **never interactive**: no hover, no pointer, no click target.
A progress row that looks pressable becomes a control that does nothing.

| State | Dark | Light | Light ratio on `--bg` |
|---|---|---|---|
| done | green **4** fill, green **11** ink | green **4** fill, green **11** ink | measured |
| failed | red **3** fill, red **11** ink | red **3** fill, red **11** ink | measured |
| current | no fill, `--ink`, a 2px brand rule | no fill, `--ink`, a 2px brand rule | the rule is the brand fill in each scheme |
| pending | transparent, gray **12** ink | transparent, gray **12** ink | measured |
| unknown | transparent, `--muted` ink, `○` | same | — |

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

## Button weight — three tiers

Weight is carried by **colour and fill, never by size**: all three tiers are the
same height and sit level on a row.

| Tier | Treatment |
|---|---|
| primary | `--brand-acid` fill, `--on-acid` label |
| secondary | `--surface` fill, `--outline` hairline |
| tertiary | no chrome, a `--muted` label and a chevron |

All three tiers are the layer's own `Button`, so their hover, active, disabled
and loading states come from `Button.module.css` and nothing else.

The fill and its label are not chosen per call site: the tier rules read
`--brand-acid` and `--on-acid` from `tokens.css`, and the pair is asserted there.
A fill and an ink taken from different tokens is what once put near-white text
on the filled user bubble (about 1.3:1, in the dark scheme only). The bubble is
text now, so that assertion moved with the fill -- but the lesson is the one
this file is for: a fill and its ink have to be a measured pair, or they are
accidentally correct in one scheme and wrong in the other.

## Type

Two tracking tokens, because they are two decisions: `--tracking-label` (0.12em,
positive) for uppercase micro-labels — a session kind, a pin, a status — and
`--tracking-display` (-0.02em, negative) for the wordmark. One value used for both
would have been a coincidence wearing a system's clothes.

Uppercase is a rule rather than a typed string: `Chip` takes `upper`, which sets
`text-transform` and the label tracking in CSS. It is scoped to labels — a small
chip is just as likely to hold a path, and `PA5` is a token before it is a word.

| Surface | Token | Stack |
|---|---|---|
| GUI | `--ff-sans` | `'Geist Variable', system-ui, -apple-system, 'Segoe UI', sans-serif` |
| GUI | `--ff-mono` | `'JetBrains Mono Variable', 'Cascadia Code', Consolas, monospace` |
| Web | `--font-sans-stack` | `'Inter', 'Noto Sans SC', system-ui, -apple-system, 'Segoe UI', sans-serif` |
| Web | `--font-mono-stack` | `'JetBrains Mono', 'Noto Sans SC', 'Cascadia Code', Consolas, monospace` |

UI text and code are separate. The GUI previously set the whole interface in a
monospace, which made every label shout.

**The GUI bundles its two faces and names no CJK one, on purpose.**
`@fontsource-variable/*` carries Geist and JetBrains Mono; Chinese is left to
`system-ui`, because a bundled CJK face is megabytes of glyphs the system already
has. The cost is that Chinese renders slightly differently on each platform, which
is accepted rather than overlooked.

**Neither GUI stack names Inter, and that decision has a test behind it.** Inter was
declared for two redesigns and never shipped, so for a while the app rendered the
system font while its tokens claimed otherwise; `styles/__tests__/tokens-css.test.ts`
fails when someone reaches for the familiar name again. Loading a font and naming one
are different acts.

Web is the opposite case: Inter arrives through `next/font`, so its stacks name it,
and `'Noto Sans SC'` is listed explicitly so Chinese does not fall back to Microsoft
YaHei and sit visibly wrong beside the Latin text. Its variables are `--font-*-stack`
rather than `--font-sans`/`--font-mono` for a reason worth keeping: a `font-family` on
`:root` would silently override what `next/font` set up in `app/layout.tsx`.

## Radius

| Token | Value | Use |
|---|---|---|
| `--r-chip` | `4` | Badges, chips, tooltips |
| `--r-control` | `6` | Inputs, buttons, selectable rows |
| `--r-panel` | `8` | Cards, panels, modals — and the logo tile, the other shape that shares this corner |
| `--r-round` | `999` | Pills, dots, avatars |

Not one value everywhere: a small chip needs to stay crisp, a large panel needs
softness. `0` everywhere reads as unfinished rather than deliberate; `12` on a dense
panel reads as loose, and `12` on a control starts to look like a pill.

**A fourth tier named `tile` (8) is gone, and so is `brand` (0).** `tile` only ever fed
an antd component token, so a rounded 16px tile could sit next to a 12px cut corner —
the "five competing boxes" problem again. The logo tile takes `--r-panel` now and there
is nothing left to keep in step. `brand` (0) meant "the brand anchor stays hard-edged",
which was the neo-brutalist frame speaking: once the black outline around a card went
away, a 0 on that card read as an unfinished box rather than a deliberate one, so the
tiles round like everything else and the tier is named for what it is.

Applied by element, not by habit: badges and chips take `--r-chip`, alerts, inputs,
buttons and selectable rows take `--r-control`, cards and panels take `--r-panel`.

`gui/src/styles/__tests__/no-literal-tokens.test.ts` enforces the token layer, not just
the radius: no literal corner radius, no literal hex colour and no hand-written font
stack anywhere in the TypeScript under `gui/src`. The claim that there is one place a
design value is written down decays one inline literal at a time, so it is asserted
rather than trusted.

What it does **not** read is the stylesheets — it globs `.ts`/`.tsx` only. A bare `px`
in a `.module.css` is legal by design, because a component's own padding is not a design
token; a colour there is expected to be a `var(--…)` reference. That half is convention
rather than a gate, which is worth knowing before trusting a green suite to have caught
it.

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
