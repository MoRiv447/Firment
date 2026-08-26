# SBC Edge-Model Deployment Manual (from zero to acceptance)

> Target platform: Debian-family SBC + systemd (battle-tested on a Radxa
> Cubie A7A). Goal: a fresh SBC + a PC + one MQTT node firmware → all-green
> `firm --doctor --sbc`. Every command below was verified in a real
> deployment; see §7 for the incident-derived troubleshooting table.
>
> Chinese edition: [docs/sbc-setup.zh-CN.md](sbc-setup.zh-CN.md).

## 1. Architecture overview

```
┌─────────┐  WiFi/MQTT   ┌──────────────── SBC ────────────────┐   LAN    ┌────── PC ──────┐
│ Node fw  │─────────────▶│ mosquitto :1883 ◀── sbc-guard       │◀────────▶│ firm CLI / GUI │
│ ESP32-S3 │  telemetry/  │        ▲                │            │  config  │  provider:     │
│ (node)   │  cmd/state   │        │                ▼            │          │  sbc-ollama    │
└─────────┘              │        └──────── ollama :11434 ──────│─────────▶│  task subagents│
                         │                   qwen2.5:0.5b class. │  /v1 API └────────────────┘
                         └──────────────────────────────────────┘
```

| Component | Role | Form |
|---|---|---|
| mosquitto | MQTT broker, the data-plane hub | apt package, systemd |
| ollama | On-device model server (OpenAI-compatible `/v1`) | official installer, systemd |
| sbc-guard | Telemetry collection, rule pre-filter, small-model classification, two-phase alerting, heartbeat | `guardd.py` + systemd (`sbc-guard.service`) |

Topic conventions (everything under the `firment/` namespace):

```
firment/device/<node>/telemetry   node → SBC telemetry lines
firment/device/<node>/alert       SBC → PC alerts (QoS1)
firment/device/<node>/state       node status frames (incl. command acks)
firment/device/<node>/cmd         PC → node commands (JSON envelope v1)
firment/guard/status              guard heartbeat (retained, 10-minute beat)
```

## 2. Prerequisites

- SBC: Debian family, Python ≥3.9, internet access (packages/model pulls),
  2GB+ RAM
- PC: firm CLI installed (`firm install`), same LAN as the SBC
- Network: **strongly recommended — pin the SBC's MAC to a static DHCP
  reservation in the router first.** IP drift breaks broker, ollama AND
  firmware config simultaneously; it bit us twice (.6→.8).

## 3. SBC-side deployment

### 3.1 mosquitto

```bash
sudo apt install -y mosquitto mosquitto-clients
sudo tee /etc/mosquitto/conf.d/firment.conf <<'EOF'
# Plain-text LAN listener: nodes and the PC are inside the LAN boundary.
listener 1883 0.0.0.0
allow_anonymous true
EOF
sudo systemctl restart mosquitto && sudo systemctl enable mosquitto
```

> Plain text inside the LAN is a deliberate trade-off — simplest path when
> node firmware carries no TLS stack. If the broker must span VLANs,
> switch to 8883 + credentials and update the PC `[mqtt]` block and the
> firmware constants accordingly.

### 3.2 ollama + models

```bash
curl -fsSL https://ollama.com/install.sh | sh
systemctl status ollama                    # ships its own unit; expect active
ollama pull qwen2.5:0.5b                   # ~400MB — guard classifier (production default)
ollama pull qwen3.5:0.8b                   # ~600MB — for async task subagents (thinking model)
curl -s http://127.0.0.1:11434/v1/models   # should list both models just pulled
```

Division of labor (measured, not guessed):

- **qwen2.5:0.5b**: non-thinking, JSON-format-reliable, fast on CPU → the
  guard's classifier. ⚠ Its semantic severity judgment is NOT trustworthy
  (benchmark sev 0/10) — in production the rule layer is authoritative and
  the LLM only refines summary/category, see §7-P6.
- **qwen3.5:0.8b**: higher quality but emits long `<think>` streams that eat
  the max_tokens budget → fine for async task subagents, wrong for realtime
  classification.

### 3.3 The sbc-guard daemon

```bash
sudo apt install -y python3-pip
# paho must be >=2.0 (v1 lacks CallbackAPIVersion); tomli only on Python<3.11
pip3 install --user "paho-mqtt>=2.0" requests "tomli; python_version < '3.11'"
mkdir -p ~/sbc-guard
```

Push three files from the PC (**never** inline long commands over ssh —
Windows quote-escaping is a minefield; scp files, then execute):

```powershell
scp sbc-guard/guardd.py,sbc-guard/rules.toml,sbc-guard/config.toml radxa@<SBC-IP>:sbc-guard/
scp sbc-guard/sbc-guard.service radxa@<SBC-IP>:sbc-guard/
ssh radxa@<SBC-IP> "sudo cp ~/sbc-guard/sbc-guard.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now sbc-guard"
journalctl -u sbc-guard -f          # expect "[guard] up ... rules=N" and "[mqtt] connected rc=Success"
```

Key `config.toml` fields (the daemon talks to the broker over loopback, so
it survives IP drift untouched):

```toml
broker_host = "127.0.0.1"     # guardd is co-located with the broker
[ollama]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "qwen2.5:0.5b"        # classifier
[guard]
standby_minutes = 10          # heartbeat beat; doctor judges freshness at 2× this value
escalate_sev = "warn"         # alerts at/above this level escalate to the desktop
```

## 4. PC-side configuration

`firm config` opens the global config (`%APPDATA%\firment\config.toml`);
add two blocks:

```toml
[mqtt]
broker = "<SBC-IP>:1883"

[providers.sbc-ollama]
type = "openai"
base_url = "http://<SBC-IP>:11434/v1"
model = "qwen3.5:0.8b"        # default model for task subagents
```

- Key: local ollama needs no auth; if your endpoint does, run
  `firm --set-key sbc-ollama sk-xxx`
- In an agent session, asking for "a small model to classify X" makes the
  AI call the `models` tool first (both qwen2.5:0.5b and qwen3.5:0.8b show
  up), then dispatch via `task {provider, model}`

## 5. Node firmware

Template: `docs/examples/esp32c3-mqtt-node/esp32c3-mqtt-node.ino` (works on
C3/S3 SuperMini boards). Build-time constants to edit before flashing:

| Constant | Meaning | Example |
|---|---|---|
| `WIFI_SSID` / `WIFI_PASS` | 2.4GHz WiFi | `"lab"` / `"…"` |
| `MQTT_HOST` | **The SBC's IP** (not loopback! update when IP drifts) | `"192.168.1.8"` |
| `NODE_NAME` | Node name = board name = pinmap board name = workbench.toml `[devices]` key | `"s3-node-1"` |

Protocol v1 essentials: nodes advertise capabilities (caps frame) on boot;
PC sends JSON envelopes `{"v":1,"cmd":"rgb.set","params":{...},"id":...}`;
nodes reply with a state frame carrying the same id as ack.

## 6. Acceptance criteria

```text
$ firm --doctor --sbc
  ✓ [mqtt] broker = 192.168.1.8:1883
  ✓ tcp 192.168.1.8:1883 reachable
  ✓ mqtt CONNACK
  ✓ guard heartbeat fresh (age 45s, beat 10min)
  ✓ sbc-ollama: model 'qwen3.5:0.8b' ready (2 served)
  ✓ devices bound: s3-node-1
```

All six ✓ = accepted. Every failure prints a fix hint; cross-reference §7.

## 7. Troubleshooting (real incidents)

| # | Symptom | Root cause → fix |
|---|---|---|
| P1 | tcp refused | mosquitto not running: `sudo systemctl status mosquitto`; confirm conf.d listens on 1883 |
| P2 | heartbeat STALE | guardd crash-looping: `journalctl -u sbc-guard -n 20`. Historical root causes: paho v1 missing `CallbackAPIVersion`; `clean_session=False` without explicit client_id (an explicit id is mandatory) |
| P3 | unit not found | The unit is named **`sbc-guard.service`**, not firment-guard |
| P4 | service can't parse config after editing via ssh | Windows editors wrote a UTF-16 BOM. Always UTF-8 no BOM: PowerShell `[IO.File]::WriteAllText($path,$t,(New-Object Text.UTF8Encoding($false)))` |
| P5 | ollama probe timeout | Cold start ~70s (model load into RAM); retry once verbatim before digging |
| P6 | revised alert severity is nonsense | qwen2.5:0.5b semantic sev is untrustworthy (benchmark 0/10). Rule-layer severity is authoritative; LLM output is summary/category refinement only. Fine-tune corpus accumulates in `~/sbc-guard-data/pairs/` |
| P7 | qwen3.5 replies come back empty | thinking stream exhausts max_tokens → budget generously (guardd sets 800 and strips `<think>` blocks) |
| P8 | everything dies at once | 9 times out of 10 it's DHCP drift: make the router reservation (§2), then sync three places — PC `[mqtt]`, provider base_url, firmware `MQTT_HOST` |
| P9 | sudo hangs over ssh | non-interactive sudo wants a password: set up SSH key auth + a `NOPASSWD` whitelist entry, or run interactively |
| P10 | pip installed but import still fails | Python≥3.11 should NOT get tomli; paho must truly be v2 (`pip3 show paho-mqtt`) |

## 8. Appendix: naming & service standards

| Object | Standard | Example |
|---|---|---|
| Node name | `<board>-node-<n>`, identical in four places (firmware NODE_NAME / pinmap board / `[devices]` key / alert node field) | `s3-node-1` |
| SBC providers | `sbc-` prefix + backend name; the models tool auto-identifies edge endpoints by base_url host == broker host | `sbc-ollama` |
| systemd unit | `sbc-guard.service`, `Restart=always RestartSec=5`, `MemoryMax=256M` as runaway containment | |
| Fine-tune corpus | `data_dir/pairs/<YYYYMMDD>.jsonl`, tuple `(line, rule_sev, llm_*, published_sev)` | |

---
*Maintenance note: keep this document in lockstep with the code — when the
guard deployment shape, topic conventions, or protocol version change,
update the relevant sections and the `firm --doctor --sbc` hint strings.*
