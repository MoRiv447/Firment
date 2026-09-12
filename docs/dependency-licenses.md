# Dependency licences — Rust

Firment is MIT. That is a claim about the four crates in this repository, and it
says nothing about the ~300 crates they are built from, each of which has its own
terms. This is the inventory, and it exists so a release does not have to take
"all our dependencies are MIT/Apache" on faith.

```bash
node docs/dependency-licenses.mjs        # reads Cargo.lock + the local registry
```

The script resolves each crate's licence **offline**, from the `license` field in
the copy of its manifest that cargo already put on disk. It needs no network and
no `cargo-license`, because a check that has to be installed before it can answer
"is anything copyleft in here?" is a check that does not get run. Re-run it after
any dependency change; the numbers below are a snapshot, not a promise.

## The shipped set, measured 2026-09-12

`cargo tree -p firment-cli --target x86_64-pc-windows-msvc` — **299 distinct
crates**. Every one of them resolved.

`Cargo.lock` holds 414 entries: 4 workspace members, 410 from the registry. The
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
| 198 | `MIT OR Apache-2.0` |
| 79 | `MIT` |
| 18 | `Unicode-3.0` (the ICU4X family — text segmentation and normalisation) |
| 16 | `MIT/Apache-2.0` — the same dual grant, older spelling |
| 13 | `Apache-2.0 OR MIT` |
| 7 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| 6 | `Unlicense OR MIT` |
| 6 | `Zlib OR Apache-2.0 OR MIT` |
| 5 | `BSD-3-Clause` |
| 5 | `Apache-2.0 OR ISC OR MIT` |
| … | 20 further dual/triple expressions, all with a permissive option |
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

17 registry crates are in `Cargo.lock` but in no shipped build:

```
arbitrary  foreign-types  foreign-types-shared  interpolate_name
libfuzzer-sys  openssl  openssl-macros  openssl-probe  openssl-sys
ppv-lite86  rand 0.9.5  rand_chacha  rand_core  security-framework
system-configuration  system-configuration-sys  vcpkg
```

They are not a gap in the inventory. `cargo tree` for the Windows target
contains none of them:

* `openssl*`, `vcpkg`, `foreign-types*` — the OpenSSL backend of `native-tls`.
  It is not what this target uses: the build pulls `native-tls` **and**
  `schannel` (Windows' own TLS) rather than `openssl`. Note that the build also
  contains `rustls`, so "which TLS backend" is not a single answer here.
* `security-framework*`, `system-configuration*` — macOS.
* `rand 0.9.x`, `rand_chacha`, `rand_core`, `ppv-lite86` — superseded entries:
  the shipped tree contains **no `rand` at all**, at any version.
* `arbitrary`, `libfuzzer-sys`, `interpolate_name` — fuzzing and macro internals.

A Linux release **will** pull the `openssl` family, and those crates are among
the 17 this offline method cannot read. Re-running the script on a machine that
has built for that target is what resolves them; the script refuses to guess a
licence from a different version of the same crate, which is the right failure.

## What this does not cover

* **Security advisories.** This is licences only. The other half of the review is
  `cargo audit`, which needs the advisory database and therefore the network —
  it is not installed on this machine. Until it is run, nothing here says
  anything about known vulnerabilities in the dependency set.
* **The JavaScript side.** `gui/` and `web/` have their own, much larger trees
  (npm). They are covered by neither this document nor this script.
* **Licences of fonts and assets.** The type stack names JetBrains Mono and
  Inter; the web client loads the logo from `public/`. Those are separate
  questions.
