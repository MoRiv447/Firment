# Firment embedded benchmark (2026-10)

A 14-case, **hardware-in-the-loop** evaluation: every case has to build, flash, run, and be
**measured on the board** (logic analyser, serial, debug probe). It does not ask whether the code
looks right; it asks whether the waveform is.

This directory is the judge itself: tasks, scoring rules, fixture drivers, the assertion engine,
and the report from the run that was actually made.

## Layout

| Path | What it is |
|---|---|
| `specs/caseNN.toml` | per-case time box, weight, steps, assertions and thresholds |
| `engine/` | the judge (step runner, assertions, static rules, report writer) |
| `hal/` | hardware abstraction: serial bus, fixture GPIO, logic analyser, probe |
| `grader/` | the interface for human / model rulings |
| `cli.py` / `campaign.py` | grade one case / run a whole round |
| `FROZEN.md` + `frozen-manifest.json` | the frozen scoring revision (currently `r7`, 27 files) |
| `freeze_check.py` | verifies the judge has not drifted from that manifest |
| `RESULTS.md` | the run: per-case results, which build they belong to, and the boundary statement |
| `ai-rulings.md` | every point a machine cannot decide, with the ruling and how to re-check it |

## Rig

- **DUT**: NUCLEO-G431RB (STM32G431RB, on-board ST-Link/V3 → virtual COM port)
- **Fixture**: a CH340's DTR/RTS drive the button lines (the two are cross-coupled — see the note in
  `hal/gpio.py`)
- **Analyser**: fx2lafw via `sigrok-cli`, four channels on PA8 / PA5 / PB8 / PB9
- **Needs a human**: case09 needs a multimeter; case03 / 12 / 14 need someone to look at the board

Ports are resolved by VID/PID + description rather than a hard-coded COM number; when two boards of
the same model are attached, name the port in `config.toml`.

## Running it

```bash
cp config.example.toml config.toml     # fill testset / judge / reports
python cli.py caps                     # ports, probe, analyser
python cli.py readiness                # per-case readiness
python -m campaign firm case01 case04 case05 case06 case07 case08 case10 case11
```

For each case `campaign` wipes the workspace, hands the task to the agent under test, flashes,
applies the stimuli in the spec, captures, grades, and writes a report. The report names the judge
revision and the measured value behind every assertion.

## What was and was not measured

- **Ran 8 cases** (01 / 04 / 05 / 06 / 07 / 08 / 10 / 11 — 57 of the 100 weight).
- **Did not run 6 cases** (02 / 03 / 09 / 12 / 13 / 14 — 43 of the 100 weight):
  - 12 and 14 need a camera pointed at the board, which the rig here does not have;
  - 13 needs the Bluetooth peer online;
  - 03 and 09 need a person to answer one prompt while the run is in progress (09 also needs a
    multimeter reading).
- Unmeasured cases are **not** counted in the denominator, and the report does not pretend they were.

## Scoring

`FROZEN.md` records every change from `r1` to `r7`, why it was made, and what it can affect. The
current revision is `r7`. `python freeze_check.py` compares the merkle of 27 files and complains if
any of them moved — including when the person moving them was me.

The rule for changing a rule: **disprove it first**. Every widening or tightening has to cite a real
failing report and say whether it could now let a real defect through.

## Known gaps in the judge

- `eval_int` does not include CMSIS headers, so a page index computed from `FLASH_BASE` is graded red
  (see the case10 ruling in `ai-rulings.md`).
- `case02.F3` hard-codes `channel = "PA8"` while the task allows any pin with a timer channel; a
  choice like PA6 cannot be measured on this wiring. Not fixed yet.

## Lab notes

`ai-rulings.md` is the notebook where each machine-undecidable point was argued, in the language the
argument happened in (Chinese). Every entry carries the report id, the `file:line` it rests on, and a
one-line command to re-check it. The tables and identifiers in it are unambiguous without the prose.
