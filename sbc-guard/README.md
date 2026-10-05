# sbc-guard

SBC-side collector + deterministic guard (docs/sbc-agent.md §3). Python 3,
two deps, no build step.

> Full from-zero deployment walkthrough (broker, ollama, PC config,
> acceptance via `firm --doctor --sbc`, troubleshooting table):
> **[docs/sbc-setup.md](../docs/sbc-setup.md)**. This file stays as the
> component-level quick reference.

## Deploy (on the Cubie A7A)

```bash
ssh radxa@192.168.1.6
sudo apt install -y python3-pip
# paho>=2.0 (v1 lacks CallbackAPIVersion); tomli only on Python < 3.11
pip3 install --user "paho-mqtt>=2.0" requests "tomli; python_version < '3.11'"
mkdir -p ~/sbc-guard
# from the PC:
scp sbc-guard/{guardd.py,rules.toml,config.toml} radxa@192.168.1.6:sbc-guard/
```

Enable + start:

```bash
sudo cp sbc-guard.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now sbc-guard
journalctl -u sbc-guard -f          # watch it work
```

## Verify end-to-end (from the PC)

```powershell
# 1) inject a fake error log line as if it came from a node
mosquitto_pub -h 192.168.1.6 -t firment/device/s3-node-1/telemetry `
  -m '{"node":"s3-node-1","kind":"telemetry","payload":"E (123) wifi: reconnect failed"}'
# 2) expect an alert back within seconds
mosquitto_sub -h 192.168.1.6 -t 'firment/device/+/alert' -C 1 -W 10000
# 3) guard heartbeat (retained — arrives immediately)
mosquitto_sub -h 192.168.1.6 -t firment/guard/status -C 1 -W 3000
```

## Test

```bash
python3 test_guardd.py        # stdlib only; no paho/ollama needed
```

Four cases over the daemon's startup failure paths — a broken `rules.toml` must warn and keep
collecting, a broken `config.toml` must exit 2 with the reason, the healthy path must still come
up, and a misspelled `[[rules]]` table must say that nothing will escalate. It runs `guardd.py` as
a real subprocess with stub `paho`/`requests` modules in a temp tree, so it measures the daemon
rather than a copy of its logic.

## Config

`config.toml` next to guardd.py:

| key | default | meaning |
|---|---|---|
| broker_host/port | 127.0.0.1:1883 | local mosquitto |
| data_dir | ~/sbc-guard-data | full-frame daily JSONL sink |
| rules_file | rules.toml | pre-filter regexes |
| [ollama] enabled | false | classify hits via ollama |
| [ollama] model | qwen2.5:0.5b | NON-thinking classifier (see note) |
| [ollama] timeout_s | 60 | per-attempt budget; ×2 attempts bounds one item |
| [guard] standby_minutes | 10 | heartbeat cadence |
| [guard] queue_max | 256 | refinement backlog ceiling (see below) |

**A broken file, held two different ways.** `config.toml` that does not parse makes the daemon
exit 2 with the reason on stderr, and the unit's `RestartPreventExitStatus=2` makes systemd record
a failure instead of restart-looping it — a loop would never reach `subscribe`, so a typo would
take the data plane down, not just the alerts. `rules.toml` that does not parse is different: the
guard prints the reason and falls back to the built-in rules, because a collector that keeps
collecting beats one that exits. A `rules.toml` with a misspelled table (`[[rules]]` instead of
`[[rule]]`) also says so — an empty rule list means nothing ever escalates.

**Refinement backlog.** The classifier runs on one worker thread, and the queue feeding it is
bounded by `[guard] queue_max`. When it is full the newest refinement request is dropped and
counted in the heartbeat as `llm_dropped`; the raw alert for that hit has already been published,
so nothing on the wire is lost, and the counter is how you can tell refinement is being shed.

**Model note**: the classifier should be a NON-thinking instruct model
(qwen2.5:0.5b works well). qwen3.5:0.8b always emits long `<think>` streams
that starve content tokens on this CPU-only SBC and make classification take
minutes; keep it for async `task` subagent work from the big model instead.

Turning `[ollama] enabled = true` routes every hit through the classifier for
sev/summary; schema-invalid replies retry once then fall back to the raw hit.
Classification runs on a worker thread so the MQTT loop never blocks.
