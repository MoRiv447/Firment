# Dependency licences — Rust

Firment is MIT. That is a claim about the four crates in this repository, and it
says nothing about the 477 packages the root workspace resolves and the 671 in
the Tauri application's own lockfile — two independent resolutions that overlap,
so neither count contains the other
(`grep -c '^name = ' Cargo.lock gui/src-tauri/Cargo.lock`), each package with its
own terms. This is the inventory, and it exists so a release does not have to
take "all our dependencies are MIT/Apache" on faith.

```bash
node docs/dependency-licenses.mjs        # reads Cargo.lock + the local registry
```

The script resolves each crate's licence **offline**, from the `license` field in
the copy of its manifest that cargo already put on disk. It needs no network and
no `cargo-license`, because a check that has to be installed before it can answer
"is anything copyleft in here?" is a check that does not get run. Re-run it after
any dependency change; the numbers below are a snapshot, not a promise.

## The shipped set, measured 2026-11-21

`cargo tree -p firment-cli --target x86_64-pc-windows-msvc` — **356 distinct
crates**. Every one of them resolved.

`Cargo.lock` holds 477 package entries: 4 workspace members, 473 from the registry. The
difference is crates the lock contains but this build never compiles (see
"Lock-only entries" below).

`--target` is not optional in that command and neither is it in this document:
a Linux or macOS release pulls a different dependency set, so the check has to
be re-run per shipped target rather than once for the repository.

## Distribution

The overwhelming majority are dual-licensed `MIT OR Apache-2.0`, which is the
Rust ecosystem's default and means the user may pick either. Roughly:

| Count | Licence |
|---|---|
| 234 | `MIT OR Apache-2.0` |
| 101 | `MIT` |
| 24 | `Apache-2.0 OR MIT` |
| 21 | `MIT/Apache-2.0` — the same dual grant, older spelling |
| 18 | `Unicode-3.0` (the ICU4X family — text segmentation and normalisation) |
| 10 | `Unlicense OR MIT` |
| 7 | `Zlib OR Apache-2.0 OR MIT` |
| 7 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| 5 | `BSD-3-Clause` |
| 4 | `Apache-2.0` |
| … | 25 further dual/triple expressions, all with a permissive option |
| 2 | **`MPL-2.0`** — see below |

Every expression above leaves a permissive option, so a binary may be shipped
under MIT with the usual notice obligations. `Unicode-3.0` and `Unicode-DFS-2016`
are the ICU4X data licences, which are permissive and require the notice.

## The two that are not simply permissive: MPL-2.0

| Crate | Version | Reached how |
|---|---|---|
| `serialport` | 4.9.0 | **direct** — `firment-tools` and `firment-cli` both depend on it |
| `option-ext` | 0.2.0 | transitive — `firment-core` → `dirs` → `dirs-sys` |

MPL-2.0 is **file-level** copyleft. It does not reach Firment's own source the
way the GPL would: shipping these crates unmodified inside a larger MIT work is
explicitly permitted. What it does require is that if we ever *modify* a file
covered by MPL-2.0, that file's source has to be published under MPL-2.0.

Neither is patched here, so today the obligation is a notice and nothing else.
The reason this is written down rather than assumed: a future patch to
`serialport` — vendoring a fix for a board that enumerates badly, say — would
change the answer, and it would do so silently.

The one that would matter most if it changed is `serialport`, because it is a
*direct* dependency rather than a detail of `dirs`. It is also the crate the
serial layer (and therefore the whole embedded half) is built on, which makes
patching it a tempting thing to do.

## Lock-only entries

102 registry crates are in `Cargo.lock` but in no shipped Windows build. They are not a
gap in the inventory — `cargo tree` for the target contains none of them — and they fall
into four groups rather than being an accident:

| Group | Examples |
|---|---|
| Other platforms | `objc2*`, `core-foundation*`, `security-framework*`, `mach2` (macOS); `libudev*`, `nix`, `x11rb*`, `linux-raw-sys` (Linux); `android_system_properties`; `wasm-bindgen*`, `web-sys`, `js-sys`, `wasi`, `wit-bindgen`; `windows_*` for every non-msvc architecture |
| A TLS stack this target does not use | `rustls`, `rustls-webpki`, `ring`, `tokio-rustls`, `hyper-rustls`, `openssl`, `openssl-sys`, `vcpkg`, `foreign-types*` |
| Test and fuzzing support | `arbitrary`, `libfuzzer-sys`, `interpolate_name`, `assert-json-diff`, `wiremock`, `bumpalo` |
| Superseded or feature-gated | `rand 0.9.x`, `rand_chacha`, `rand_core`, `ppv-lite86`, `lazy_static`, `tinyvec`, `jiff*`, `httpdate`, `signal-hook*` |

Two corrections to what this section said before:

- It claimed the build "also contains `rustls`". **It does not.** `reqwest` is built with
  `native-tls` (`schannel` on Windows), and the only rustls artifact in the shipped tree is
  `rustls-pki-types` — type definitions, not a backend. `rustls` and `rustls-webpki` are
  lock-only on this target, which is also why `cargo tree -i rustls-webpki` prints nothing
  here while `cargo audit` still sees it.
- The count was 17 and is 102 now. The growth is probe-rs's tree (`defmt`,
  `portable-atomic`, `objc2`, the wasm crates) rather than anything this project added
  directly.

A Linux release **will** pull the `openssl` family and both rustls crates. Re-running the
script on a machine that has built for that target is what resolves them; the script refuses
to guess a licence from a different version of the same crate, which is the right failure.

## What this does not cover

* **Security advisories.** This document is licences only. The other half of the
  review is `cargo audit`; `.cargo/audit.toml` holds the policy, including one
  recorded ignore (`ruzstd 0.8.3`, with the reason and the reachability argument
  written next to it). CI runs it against **both** lockfiles — the workspace one
  directly, the Tauri one with `--file`, because cargo-audit reads its config from
  the directory it runs in and does not walk up to the repository root. The same
  two commands work locally when the network is up:
  `cargo audit` and `cargo audit --file gui/src-tauri/Cargo.lock`.
  Re-check it by running those, not only by pushing.
* **The JavaScript side.** `gui/` and `web/` have their own, much larger trees
  (npm). They are covered by neither this document nor this script.
* **Licences of fonts and assets.** The type stack names JetBrains Mono and
  Inter; the web client loads the logo from `public/`. Those are separate
  questions.
