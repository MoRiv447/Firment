# Results: Firment × Claude Code, 2026-10-18 (all three rounds listed)

> **No best-of, no mean, no median** — every round is listed so the spread is visible.
> Each case is scored **0–5 (machine verdict + ruling)**. case05 excludes H2 by the rubric's own
> rule (max 4); case10 skips F2 by the frozen convention (max 4).

## Conditions

| | |
|---|---|
| Rig | NUCLEO-G431RB + CH340 fixture + fx2lafw analyser |
| Agent config | both sides `deepseek-flash` + `thinking=high`, verified per run in each report |
| Cases | **8 / 14** (01/04/05/06/07/08/10/11); 02/03/09/12/13/14 were **not run — 43/100 of the weight** |
| Judge | r6; case10 re-graded with r7 (r7 only widens the symbolic page-index form) |
| Binary | main board `40bc50adc844526fa768ab2c69bff086`; **case04 uses the release build `742fa4c23d954d0879836525d6418375`** (see "Build note") |
| Fixture | the fixture holds the button lines released for the whole agent session (measured effect on the line level: **none** — `IDR=1` either way) |
| Rounds | 2026-10-18 11:05–17:30, each side a fresh session per round, workspace wiped |

## Per case (every round listed)

| Case | Weight | firm R1 | firm R2 | firm R3 | claude R1 | claude R2 | claude R3 |
|---|---|---|---|---|---|---|---|
| case01 VCP hello | 5 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 |
| case04 button debounce | 8 | **5.0**¹ | **2.0**¹ | **5.0**¹ | **5.0** | **5.0** | **2.0** |
| case05 precision PWM | 6 | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 |
| case06 capture self-test | 7 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 |
| case07 ring buffer | 8 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 |
| case08 DMA+IDLE | 9 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 |
| case10 flash counter | 7 | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 |
| case11 IWDG | 7 | 5.0 | 5.0 | 5.0 | 5.0 | 5.0 | **4.0** |
| **Weighted total (/57)** | | **54.4** | **49.6** | **54.4** | **54.4** | **54.4** | **48.2** |

¹ firm's three case04 numbers come from three independent runs of the **release build** (short-press
counts 10 / 9 / 10), mapped in order to R1/R2/R3; they are not the same run as the other cases. The
old build scored 5.0 / 2.0 / 3.0 on this case.

**The only cases that moved are case04 and claude's case11.** The other six were identical across
all 18 runs.

## What actually happened in case04

**firm (three runs of the release build)**

| Run | Score | Short / long | Why |
|---|---|---|---|
| 1 | 5.0 | 10 / 3 | read `nucleo.c:256..375`, saw `:284 Pull = GPIO_PULLDOWN`, concluded active-high |
| 2 | **2.0** | **9 / 4** | the state machine was wrong at both ends: one short press missed, one long press invented. **Not polarity** — this run read `PUPDR PC13=pull-down` |
| 3 | 5.0 | 10 / 3 | as run 1 |

**claude (three rounds)**

| Round | Score | Why |
|---|---|---|
| R1 | 5.0 | read the BSP, concluded active-high |
| R2 | 5.0 | same |
| R3 | 2.0 | the code comment says "B1 (PC13, **active low** on the NUCLEO-G431RB)" and configures `PULLUP` → 0 bytes of output |

> On the old build firm scored 5.0 / 2.0 / 3.0 here: R2 silent (wrong polarity), R3 one missed press.
> The release build removes the "whole run is silent" class (all three printed) but **the debounce
> state machine still loses a press** (run 2: 9/10 and an extra long press).
> claude's failure is the same prior both times: treating "the Nucleo's B1 is active-low" as a fact.

## Build note (why case04 is not the same binary as the rest)

The seven other cases were measured with `40bc50ad`. Before the release, four changes landed in the
**tool output layer**; the judge itself did not move (`engine/`, `specs/`, `hal/` are frozen at r7):

| Change | Where |
|---|---|
| GPIO reads decode MODER/PUPDR/IDR/ODR (layout table in the KB, F1/F4/G0/G4/H7) | `tools/gpio_decode.rs` + `cheatsheets/gpio-layouts.toml` |
| `monitor`'s empty window now says it is evidence of silence, not of a working path | `tools/monitor.rs` |
| Writing a GPIO says the level you drive is your own stimulus | `tools/debug.rs` |
| `grep` says "a hit is not a read — open the definition body" | `tools/grep.rs` |

The release build `742fa4c2` was run three times on case04: **5.0 / 2.0 / 5.0** (short counts
10 / 9 / 10). Two cases were spot-checked once each: case08 = 5.0 (same as the old board) and
case10 = 4.0/4 (same). The remaining five cases keep their old-build numbers.

**case10's H2 machine failure is a false red**: this run wrote
`COUNTER_PAGE_INDEX = (COUNTER_PAGE_ADDR - FLASH_BASE) / FLASH_PAGE_SIZE`, which evaluates to 63
(`0x0801F800`), but the rule cannot resolve the CMSIS `FLASH_BASE`. The ruling awards the point; see
`ai-rulings.md`.

## Boundaries of this result (author's statement)

1. **Mixed builds**: case04 comes from the release build, the rest from the previous one. The scoring
   path did not change, but agent behaviour was not re-measured end to end on the new build.
2. **Small n**: three runs per case on the old build, one on the spot-checks; case04 has three.
3. **8 of 14 cases**: 02/03/09/12/13/14 were not run because the camera, the Bluetooth peer or the
   human observer was not available — 43/100 of the weight.
4. **Reproducible**: the judge is frozen at r7 and ships with the repository; the wiring is described
   in each spec's `caps`. Anyone can re-run it on the same rig. If you cannot reproduce it, the
   result is wrong — open an issue.

## Re-running it

```
cd benchmark
<venv python> -m campaign firm   case01 case04 case05 case06 case07 case08 case10 case11
<venv python> -m campaign claude case01 case04 case05 case06 case07 case08 case10 case11
```

Checks before grading (none of them modify anything):

```
<venv python> cli.py caps         # ports / probe / analyser
<venv python> cli.py readiness    # per-case readiness
<venv python> freeze_check.py     # is the judge still the frozen one
```

## How to read the table

1. **Read it per case, not as a ranking.** Six cases were identical across 18 runs; the whole
   difference comes from case04 and claude's case11.
2. **A single run is not a conclusion.** Same model, same case: 5.0 → 2.0 → 5.0 for firm and
   5.0 → 5.0 → 2.0 for claude — a 3-point swing (4.8 weighted), larger than the gap between the two
   agents.
3. **If you need one number**, write: firm 54.4 / 49.6 / 54.4, claude 54.4 / 54.4 / 48.2. Do not
   combine them, and do not pick the best.

## Where the evidence is

- Per-run reports: `reports/20261018-*-{firm,claude}-case*/report.md` (not shipped — 72 MB)
- Rulings: `ai-rulings.md`, one entry per point with the report id and a re-check command
- Workspace copies: `D:\Firment-bench-archive\<agent>-<case>\run-N\` (author's machine only)
- Scoring revision: `FROZEN.md` — every change from r5 to r7 and what it can affect
