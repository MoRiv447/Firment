# Layout

The shell's geometry: how wide each column is, what happens as the window shrinks, and
which file owns each number. `tokens.md` owns colour, type, radius and motion; this file
owns where things sit.

Numbers are written with the file they live in, so a reader can check them instead of
trusting them — and so a change that moves one has an obvious place to update. Values
that are a component's own business (a drag limit, a floor) stay with that component
rather than being centralised here.

## The GUI shell (Tauri, px)

Two bars and three columns: title bar, then `rail | (transcript | workbench) |
inspector`, then the status bar.

| Region | Size | Where |
|---|---|---|
| window, default / minimum | 1280 × 820 / 980 × 640 | `gui/src-tauri/tauri.conf.json` |
| rail — sessions | **248**, fixed | `gui/src/App.module.css` (`.rail`) |
| transcript — floor | **420** | `gui/src/App.module.css` (`.pane[data-pane='chat']`) |
| inspector — default | **320** | `gui/src/App.tsx` (`localStorage['inspector-width']`) |
| inspector — drag range | **240 … 560** | `gui/src/shell/Inspector.tsx` |
| inspector — collapsed | **38** (icon rail) | `gui/src/shell/Inspector.module.css` |
| drag handle | **5** | `gui/src/shell/Splitter.module.css` |
| title bar, status bar | `--h-bar` (**44**) | `gui/src/styles/tokens.css` |

The two documented rules, in the code's own words because they are the reasons the
numbers are what they are:

- **Only the transcript has a floor.** "It is the thing you are reading, and at the
  window's minimum width a shrinking transcript wraps code blocks mid-token; the
  workbench scrolls on its own and is the pane you open on purpose" (`App.module.css`).
- **The inspector's limits are the column's business.** 240 is where a diff starts
  wrapping, 560 is where the transcript beside it stops being readable at the window's
  minimum width (`Inspector.tsx`). The dragged width survives a restart through
  `localStorage`, and the value stored there is only a starting point — the clamp
  applies on the way in (`App.tsx`).

The rail's width is **fixed and undraggable**, and that is the whole of what the code
says about it: nothing in the source justifies 248. Before the shell rewrite it was near
160 and the plan for that round asked for 200–205, so the current number was not chosen
against a stated need. Left as an open question rather than dressed up: a rail is scanned
rather than read, so its width is answerable from what a session title and its timestamp
need, and nothing currently says what that is.

## The TUI shell (ratatui, cells)

| Region | Size | Where |
|---|---|---|
| rail — sessions | **24** wide | `crates/firment-tui/src/view.rs` (`RAIL_WIDTH`) |
| evidence — ladder | **30** wide, **7** tall | same (`EVIDENCE_WIDTH`, `EVIDENCE_HEIGHT`) |
| transcript | whatever is left, floor **24** | same (`Constraint::Min(24)`) |

A terminal cannot afford all three columns at every width, so the side panels appear on
a ladder — and the order is a statement about the product, not about pixels: *"The
ladder says what has been proven, the rail says where you are, and being lost is
survivable in a way that claiming unproven work is not."* (`view.rs`)

| Body width | Columns |
|---|---|
| ≥ 100 | rail + transcript + evidence |
| ≥ 80 | transcript + evidence |
| < 80 | transcript only |

## What is shared

- Spacing, heights, radii and z-order are tokens: `--sp-*` (4px base), `--h-row` (40),
  `--h-input` (36), `--h-min` (28), `--r-*`, `--z-*`. See `tokens.md`.
- The GUI's rail and inspector widths are deliberately **not** tokens. They belong to
  the container that draws them, and the inspector's pair is enforced where the drag
  happens, so a token would be a second place to keep in step.
- The TUI's numbers are compile-time constants, and its rail is present or absent by
  width rather than shrunk: a 12-cell session list is worse than none.
