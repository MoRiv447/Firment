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
| `outline` | `#000000` | `#000000` | the neo-brutalist frame and hard shadow |

The light hover is an **acid tint, not a grey**. `muted` is 4.51:1 on `bg` and
that ground is already the AA floor, so any grey dark enough to read as a hover
pulls a muted label under it (4.40:1 at `#F4F4F5`, 4.47:1 at `#F6F6F7`). The tint
is the only candidate that holds — 4.68:1 for `muted`, 17.16:1 for `ink` — and it
reads as a weaker sibling of the solid-acid selection rather than competing with
it.

The focus ring is **not** the acid in the light scheme: acid on a light ground is
1.27:1 and a keyboard user cannot see it. The dark scheme can afford the acid ring
(15.06:1); the light scheme uses `brandInk`.

`outline` stays pure black in both schemes. It is the signature — 2px and 3px
frames plus the hard offset shadow — and it reads on either ground. The light
scheme gets its softness on controls from `line`/`lineStrong` instead
(`antdTheme` maps `colorBorder` to those), not by lightening the outline.

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

## Slant — the one signature that cannot be substituted

The cut is the brand's structural mark (the logo is three slanted bars,
`skewX(-13deg)`). On a control it is drawn with **`clip-path`, never a
`transform: skewX`**: a transform shears the type with the shape, and Latin text
leaning is a decal rather than a design.

Geometry, all from `slant` in `tokens.ts`:

| Value | Purpose |
|---|---|
| cut `12px` | horizontal run of the cut |
| gap `8px` | between adjacent slanted edges |
| `opticalPadLeft 5px` | extra left padding, see below |
| `controlHeight 40px` | every control on a row |

**The optical centre.** Cutting the bottom-left triangle removes area from the
left, which shifts the remaining shape's centre of mass about 2.5px the other way.
The label is then geometrically centred and still reads as off-centre, so it gets
5px more padding on the left than on the right.

**The cut is on the fill, not the button.** `clip-path` clips whatever it is
applied to, so clipping the button would take the label, the focus ring and the
border with it. The fill is its own layer behind the label. The dark edge is a
second clipped layer with the fill inset 1px inside it — a `border` on a clipped
element exists only on the four box edges, so the diagonal comes out with no edge
at all.

**Rounded corners never meet the cut.** `radius.panel` (8px) rounds over a 12px
diagonal and eats it; a clipped control keeps `border-radius: 0`.

### Button weight — three tiers

Weight is carried by **colour and fill, never by size**: all three tiers are
`controlHeight` tall and sit level on a row.

| Tier | Treatment |
|---|---|
| primary | `brandAcid` fill, `onAcid` label, slanted. One per screen |
| secondary | `surface` fill, `lineStrong` hairline, square corners, unslanted |
| tertiary | no chrome: `muted` label and chevron, going to `ink` on hover |

Only the primary is cut. The slant is a brand anchor, and one on every button
would dilute it into decoration.

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
| `brand` | `0` | Logo and icon tiles — the brand anchor stays hard-edged |
| `chip` | `2` | Badges, chips, tooltips |
| `control` | `4` | Inputs and buttons |
| `panel` | `8` | Cards, panels, modals |

Not one value everywhere: a small chip needs to stay crisp, a large panel needs
softness. `0` everywhere reads as unfinished rather than deliberate; `12` on a
control starts to swallow a slanted edge.

Applied by element, not by habit: badges and chips take `chip`, alerts, inputs,
buttons and selectable rows take `control`, cards and panels take `panel`, and
`brand` (0) belongs to the logo and icon tiles, whose hard edge *is* the anchor.

`gui/src/styles/__tests__/no-literal-tokens.test.ts` enforces the whole token
layer, not just the radius: no literal corner radius, no literal hex colour and
no hand-written font stack anywhere under `gui/src` outside `tokens.ts`. The
claim that there is one place a design value is written down decays one inline
literal at a time, so it is asserted rather than trusted.

## Motion

- Enter `160ms`, standard `200ms`, exit `120ms`
- Curve `cubic-bezier(.2,.8,.2,1)`
- Stagger `40ms` on grouped elements
- Everything respects `prefers-reduced-motion`
- **TUI caps at 15fps**, not 60: Windows Console, SSH and tmux repaint badly at
  high rates, and `firm` over SSH to a dev board is a normal way to work. The TUI
  also disables motion entirely under `SSH_CONNECTION`, `TERM=dumb`, or a
  non-TTY, and offers `--no-anim`.

## State semantics

Three states must be visually distinct, and **red is reserved for real errors**:

| State | Treatment |
|---|---|
| `StateUnknown` | Neutral `○`, muted ink. **Never rendered as off** — unknown is not failed |
| `StateOffline` | Muted, with the last-seen time |
| `StateError` | Only after a user action actually failed |

The rule for any new panel: **unconfigured → hidden entirely; configured but empty
→ neutral copy; only a real failure → red.**
