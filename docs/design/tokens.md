# Design tokens

The single source of truth for colour, type, radius and motion across the three
surfaces. Implementation mirrors:

| Surface | Lives in | Consumed by |
|---|---|---|
| GUI (Tauri + React) | `gui/src/styles/tokens.ts` | `ConfigProvider` via `antdTheme()` |
| Web (Next.js) | `web/src/styles/tokens.css` | CSS variables + Tailwind |
| TUI (Rust) | `crates/firment-tui/src/theme.rs` | ratatui styles, with a colour-degradation chain |

The GUI has **no stylesheet** — `gui/src` contains zero `.css` files and every
colour is an inline hex or an antd theme token. That is why the GUI's token layer
is data mapped into `ConfigProvider` rather than a CSS-variable file: CSS
variables would be invisible to antd's own components.

## Colour

Neutral grounds and neutral text, with the brand green in exactly four places:
the logo, the primary CTA, progress, and the current step. An interface tinted
green makes the diff's own red and green harder to read, and Firment's screens
are mostly diff.

| Token | Value | Use |
|---|---|---|
| `bg` | `#0F0F12` | Page background |
| `surface` | `#18181B` | Cards, panels, sidebars |
| `surfaceRaised` | `#1F1F23` | Popovers, elevated cards |
| `ink` | `#E4E4E7` | Body text |
| `muted` | `#A1A1AA` | Secondary text — **neutral grey, never olive** (olive reads as disabled) |
| `line` | `#2A2A2F` | Hairline dividers |
| `lineStrong` | `#3F3F46` | Stronger borders |

### Brand vs status green — two different colours on purpose

| Token | Value | Use |
|---|---|---|
| `brandAcid` | `#B4F779` | **Only** the logo, primary CTA fill, progress, current step |
| `onAcid` | `#15200D` | Text/icons sitting ON `brandAcid` |
| `brandInk` | `#3B6D11` | Brand green dark enough to be read AS text on a light ground |
| `successBg` | `#14532D` | Status background (passed, mainline) |
| `successInk` | `#86EFAC` | Status text |
| `successBorder` | `#166534` | Status border |

`brandAcid` is 85° (acid lime); `successInk` is 145° (true green). They are 60°
apart, so they read as different things: the brand colour is *identity*, the
status colour is *feedback*. Using one for both made "this is Firment" and "this
passed" look identical.

**`brandAcid` is ~1.8:1 on a light ground.** It is a highlighter, never a text or
icon colour. Anything readable that is green-on-light uses `brandInk`.

### Diff

| Token | Value |
|---|---|
| `diffAddedBg` / `diffAddedInk` | `#14311C` / `#86EFAC` |
| `diffRemovedBg` / `diffRemovedInk` | `#3B1218` / `#FDA4AF` |
| `diffMetaInk` | `#A1A1AA` (hunk headers, context) |

Diff colours are their own family rather than reusing the status colours: an
added line and a passed check are not the same message.

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
| `control` | `4` | Inputs, buttons |
| `panel` | `8` | Cards, panels, modals |

Not one value everywhere: a small chip needs to stay crisp, a large panel needs
softness. `0` everywhere reads as unfinished rather than deliberate; `12` on a
control starts to swallow a slanted edge.

## Motion

- Enter `160ms`, standard `200ms`, exit `120ms`
- Curve `cubic-bezier(.2,.8,.2,1)`
- Stagger `40ms` on grouped elements
- Everything respects `prefers-reduced-motion`
- **TUI caps at 15fps**, not 60: Windows Console, SSH and tmux repaint badly at
  high rates, and `firm` over SSH to a dev board is a normal way to work. The
  TUI also disables motion entirely under `SSH_CONNECTION`, `TERM=dumb`, or a
  non-TTY, and offers `--no-anim`.

## State semantics

Three states must be visually distinct, and **red is reserved for real errors**:

| State | Treatment |
|---|---|
| `StateUnknown` | Neutral `○`, muted ink. **Never rendered as off** — unknown is not failed |
| `StateOffline` | Muted, with the last-seen time |
| `StateError` | Only after a user action actually failed |

The rule for any new panel: **unconfigured → hidden entirely; configured but
empty → neutral copy; only a real failure → red.**
