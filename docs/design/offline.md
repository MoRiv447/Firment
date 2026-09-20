# Offline capability matrix — design review

Plan §5, item 10. This is the review the plan requires **before** the work starts, for the
same reason it requires the other two: the item is listed as "完全无网络可用", which is a
larger claim than it looks, and building the wrong half of it is a lot of work.

Every claim below is checked against the code, with the file named. Nothing here is from
memory — the point of a review is to be able to act on it.

---

## 1. Three different things are called "offline"

They need different answers, and picking the wrong one wastes the whole item:

| Reading | What it means in practice | Who has it |
|---|---|---|
| **A. No internet, LAN allowed** | a laptop with no uplink, next to an SBC running Ollama | **this project's own setup** — `[providers.sbc-ollama]` points at `192.168.1.8:11434` |
| **B. No network at all** | an air-gapped bench, a locked-down CI runner | a firmware lab with no wifi, a customer machine |
| **C. No *cloud* provider configured** | the user simply has not set an API key | anyone doing build/flash work only |

The plan's wording is B, and most of what B needs is already true (see §3). But the
**highest-value** case is A, because it is the one this project actually runs: a LAN model is
a local model, and `core/src/local.rs` already treats it that way (`is_private_url` counts
`192.168.`, `10.`, `172.16-31.` as local, with the reason written in the code).

**Recommendation: define the feature as A, verify it against B, and leave C as "works today
and needs no feature".** A feature that only delivers B would be a feature this project's own
user cannot exercise.

## 2. The matrix

`✓` works with no internet · `~` works with a caveat · `✗` needs the network

| Capability | Offline? | What actually happens today | Evidence |
|---|---|---|---|
| Sessions, journal, ledger, undo | ✓ | files under the session directory | `core/src/session.rs`, `core/src/journal.rs` |
| read/write/edit/glob/grep/symbols | ✓ | local filesystem | `tools/src/tools/*` |
| `rename_symbol` (transactional) | ✓ | local, journal-backed | `tools/src/tools/rename_symbol.rs` |
| build / flash / run / monitor / la / observe | ✓ | the toolchain and the probe are local | `tools/src/tools/{build,flash,monitor,la,observe}.rs` |
| Board profiles, ADRs, board/ADR digest | ✓ | compiled-in TOML and local markdown | `core/src/board.rs`, `core/src/adr.rs` |
| Static rules review (`firm review <path>`) | ✓ | pure text rules | `tools/src/review/rules.rs` |
| Evidence review (`firm review evidence`) | ✓ | reads a HIL replay log | `tools/src/review/evidence.rs` |
| `firm replay` / `firm share` | ✓ | local files | `core/src/eventlog.rs`, `core/src/share.rs` |
| **Model-backed review** (`firm review last`) | ✗ | needs a provider | `core/src/review/self_review.rs` |
| Chat / agent turns | ~ | a **LAN** provider works, a cloud one does not | `core/src/http.rs` (LAN proxy exclusion) |
| `firm review deps` — licences | ✓ | `cargo metadata` + licence files | `core/src/review/deps.rs` |
| `firm review deps` — advisories | ✗ | `cargo-audit`, and the advisory DB is a **live feed**; the local snapshot was fetched 2026-08-11 | `AGENTS.md`, "no network here" entry |
| `web_search` / `web_fetch` | ✗ | by definition | `tools/src/tools/web_{search,fetch}.rs` |
| Local model probe (`firm doctor`, `firm config`) | ✓ | localhost + LAN, **200 ms** per probe, cached 15 min | `core/src/local.rs` (`PROBE_TIMEOUT`) |
| Provider probes in `firm doctor` | ✗ | 10 s / 8 s timeouts, reports each as unreachable | `cli/src/doctor.rs:64,930` |
| Device plane (MQTT: guard alerts, `device_log`, `device_cmd`) | ~ | a **LAN** broker works; the default `broker.emqx.io` does not | `cli/src/main.rs:899`, `tools/src/tools/device_cmd.rs:123` |
| Install / update (`install.sh`, `install.ps1`) | ✗ | downloads a release | `install.sh`, `install.ps1` |
| CI advisory gate | ✗ | by design — it lives in CI *because* it needs a live feed | `.github/workflows/ci.yml` |

**No telemetry of any kind.** The only "telemetry" in the tree is *device* telemetry from the
SBC data plane (`core/src/config.rs:36`, `tools/src/tools/device_log.rs`); nothing reports
usage anywhere. This matters for an offline feature because "offline" claims are usually
about trust, and the trust claim is already true.

## 3. What is already true, and should not be rebuilt

- **No phone-home, no update check, no usage reporting.** Nothing to disable.
- **The proxy policy already understands LAN.** `http.rs` attaches the private ranges as
  `no_proxy` to every client, and the comment says why: "a machine behind a proxy cannot
  reach its own Ollama". An offline feature that re-implemented this would fight it.
- **The local-model probe is already offline-shaped**: 200 ms, cached, and run only from
  `firm config` / `firm doctor` (§16.2-6).
- **Every review capability except `last` is local**, and the plan's own tiering put the
  local ones first.

## 4. Gaps this review found (with evidence)

These are defects, not missing features — they are the difference between "works offline" and
"works offline *well*":

1. **The chat path has no timeout.** `grep -rn '\.timeout(' crates/` finds timeouts in the
   doctor probes (10 s), `list_models` (10 s), the local probe (200 ms), `models` (8 s),
   `web_fetch` (20 s), `monitor` (50 ms) — and **none in `core/src/provider/`**.
   `http_client()` (`http.rs:47`) builds a client with no timeout, so a **black-holed**
   endpoint (a dropped packet, a sleeping laptop, a wifi that is up but not routed) hangs
   until the OS gives up — which is minutes, not seconds. A **refused** connection fails
   instantly, which is why this has not been noticed: this project's failures have all been
   refuses. **Fix:** an idle/read deadline on the provider client, generous (say 120 s
   between bytes) because a slow model is not a broken one.
2. **The liveness heartbeat is parsed and dropped.** `ProviderEvent::Activity => {}`
   (`core/src/agent.rs:1143`). The parsers send it so that a stalled stream is *visible* —
   but nothing acts on its absence, so the information is collected and thrown away. The same
   deadline as (1) is where it belongs: "no bytes *and* no heartbeat for N seconds" is a
   statement only this codebase can make.
3. **Nothing tells the user what is unavailable.** `firm doctor` reports each provider as
   reachable or not, one line per provider, but there is no single answer to "can I work right
   now?" — and that is the question an offline user has. **Fix:** a `doctor` summary line
   (LAN/local model present? provider reachable? MQTT broker reachable?) rather than five
   lines the reader has to combine themselves.
4. **Offline install is undocumented.** `install.sh`/`install.ps1` download a release; a
   machine with no internet needs `cargo install --path crates/firment-cli` from a checkout or
   a copied binary. **Fix:** a short section in the install docs, and a `firm doctor` line
   that names how *this* copy was installed (it already knows: `doctor_install` compares the
   running executable with the installed one).
5. **The advisory gap is already known but not surfaced where it matters.** `AGENTS.md`
   records that the local advisory DB is a stale snapshot. In an offline review, "0
   advisories" from a stale database reads exactly like "0 advisories from a current one".
   **Fix:** the dependency review already has the concept — it distinguishes "not read" from
   "read and clean" — the advisory half needs the same treatment: say the age of the database
   it used, or say it could not check.

## 5. What this feature is *not*

- **Not a local model.** Serving weights is Ollama's job; this project's job is to notice a
  server and use it (`core/src/local.rs` does exactly that).
- **Not an air-gapped guarantee.** No amount of code makes a build that needs a crate download
  work offline; the honest deliverable is *knowing* which step needs the network, before
  running it.
- **Not a new mode to configure for most people.** A flag that has to be set correctly is a
  flag that is set wrong; the gaps in §4 are improvements that make the offline case work
  without anyone declaring it.

## 6. The work this review recommends, in dependency order

1. **A provider deadline** (§4.1, §4.2) — the only item that changes behaviour under failure,
   and the one that turns "hangs" into "reports". `core/src/http.rs` + `core/src/provider/`.
2. **A `doctor` line that answers "can I work right now?"** (§4.3) — cheap, and it is what the
   user actually asks. `cli/src/doctor.rs`.
3. **Say what the advisory check could not do** (§4.5) — the dependency review already has the
   vocabulary. `tools/src/review/deps.rs`.
4. **Document offline install** (§4.4) — `docs/`, plus one line in `doctor`.
5. **A regression test for (1)**: an endpoint that accepts a connection and then says nothing,
   asserting the call fails within a bounded time. This is the test that keeps the fix honest,
   and it runs offline — which is the whole point.

## 7. Open questions for the maintainer

1. **Is case A (LAN allowed) the definition you want?** If the real target is B, item 3 above
   turns from "one doctor line" into "an offline capability report", which is a different
   shape.
2. **How long is a slow model allowed to be silent?** 120 s is a guess; the local models in
   `[providers]` and a cloud reasoning model have very different warm-up times.
3. **Should `firm doctor` ever *fail* (exit 2) for a missing provider?** Today it does not,
   which is right for a CLI-only user and wrong for someone whose whole workflow is chat.

---

## Status

*The argument above is left as written; this section records what has moved since.*

| §4 item | Status |
|---|---|
| 1. A provider deadline | **Done** — `9436ead`: `PROVIDER_READ_TIMEOUT` (120 s between bytes) and `PROVIDER_CONNECT_TIMEOUT` (15 s) in `core/src/http.rs`, on a client built by `provider_client()`, which replaces `http_client()` — its only callers were the two providers, so the change lands exactly on the chat path. |
| 2. The heartbeat | **Done by construction** — a keep-alive is a byte, so the read deadline *is* the "no bytes and no heartbeat" rule. No extra machinery, and the `ProviderEvent::Activity => {}` arm is left alone because it no longer needs to do anything. |
| 3. A `doctor` line answering "can I work right now?" | **Done** — `78a0bcc`: a closing summary built from the probes that already ran (chat reachability, local servers, a CONNACK check on the MQTT broker), plus the line that stops an offline reader concluding that nothing works. Its wording is a pure function, so the test covers the offline case, the LAN case, and "no providers configured" without a network. |
| 4. Document the offline install path | Not started |
| 5. The regression test for (1) | **Done** — a local listener that accepts and then stays silent, asserted to fail *and* to have waited (≥250 ms), against a 300 ms deadline built through the same constructor the providers use. The lower bound is the part that distinguishes "the deadline fired" from "the connection was refused instantly". |

---

*Written 2026-09-20 against `7b5d08b`; status section updated after `9436ead` and `78a0bcc`.
If the code moves, the file:line references in §2 and §4 are the parts to re-check.*
