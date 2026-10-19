# rtt-probe-fw — the fixture that proves RTT works

A 30-line `no_std` firmware whose only job is to initialise an RTT control block and
print an incrementing line. `probe::rtt_stream` is the one piece of the probe library
that the benchmark firmware cannot exercise (all of it prints over LPUART), so this
exists to answer "does the RTT branch actually receive bytes?".

## Build, flash, verify

```bash
rustup target add thumbv7em-none-eabihf        # once
cd fixtures/rtt-probe-fw && cargo build --release

cd ../..
cargo run -p firment-tools --example probe_smoke -- \
  --elf fixtures/rtt-probe-fw/target/thumbv7em-none-eabihf/release/rtt-probe-fw \
  --rtt-ms 4000
```

Expected (measured on a NUCLEO-G431RB, 2026-10-19):

```
[ok] probes visible: 1 (122 ms)
[ok] flashed 3 segment(s) ... (270 ms)
firment-rtt-smoke 0
firment-rtt-smoke 1
firment-rtt-smoke 2
[ok] RTT 60 bytes in 4097 ms
```

## Two things this fixture taught

- `rprintln!("... {n}")` prints the literal `{n}`: rtt-target 0.6's single-string arm
  writes the string without formatting. Use `rprintln!("... {}", n)`.
- The first version of this file was inside the Firment workspace and cargo tried to
  build it for the host target. `[workspace]` in its `Cargo.toml` opts out.

It is not part of the product and not built by CI; `target/` is ignored.
