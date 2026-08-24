# sbc-guard

SBC-side collector + deterministic guard (docs/sbc-agent.md §3). Python 3,
two deps, no build step.

## Deploy (on the Cubie A7A)

```bash
ssh radxa@192.168.1.6
sudo apt install -y python3-pip
pip3 install --user paho-mqtt requests   # the only two imports
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
  -m '{\"node\":\"s3-node-1\",\"kind\":\"telemetry\",\"payload\":\"E (123) wifi: reconnect failed\"}'
# 2) expect an alert back within seconds
mosquitto_sub -h 192.168.1.6 -t 'firment/device/+/alert' -C 1 -W 10000
# 3) guard heartbeat (retained — arrives immediately)
mosquitto_sub -h 192.168.1.6 -t firment/guard/status -C 1 -W 3000
```

## Config

`config.toml` next to guardd.py:

| key | default | meaning |
|---|---|---|
| broker_host/port | 127.0.0.1:1883 | local mosquitto |
| data_dir | ~/sbc-guard-data | full-frame daily JSONL sink |
| rules_file | rules.toml | pre-filter regexes |
| [ollama] enabled | false | classify hits via ollama |
| [ollama] model | qwen2.5:0.5b | NON-thinking classifier (see note) |
| [guard] standby_minutes | 10 | heartbeat cadence |

**Model note**: the classifier should be a NON-thinking instruct model
(qwen2.5:0.5b works well). qwen3.5:0.8b always emits long `<think>` streams
that starve content tokens on this CPU-only SBC and make classification take
minutes; keep it for async `task` subagent work from the big model instead.

Turning `[ollama] enabled = true` routes every hit through the classifier for
sev/summary; schema-invalid replies retry once then fall back to the raw hit.
Classification runs on a worker thread so the MQTT loop never blocks.
