#!/usr/bin/env python3
"""sbc-guard — SBC-side collector + deterministic guard (docs/sbc-agent.md §3).

Data plane:
  subscribe firment/#  -> append EVERY frame to events-YYYYMMDD.jsonl (never drop)
Control plane:
  rules.toml pre-filters matched lines; only hits go to the small model
  (qwen via ollama, optional) for classification/summary with strict JSON
  schema; result published on firment/device/<node>/alert.
Heartbeat:
  every standby_minutes publish firment/guard/status (retained) with counters.

Run:  python3 guardd.py [config.toml]     (systemd unit in this directory)
"""

import json
import re
import sys
import time
from pathlib import Path

import paho.mqtt.client as mqtt
import requests

try:
    import tomllib
except ModuleNotFoundError:  # python < 3.11
    import tomli as tomllib

DATA_DIR = Path.home() / "sbc-guard"
CFG_PATH = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent / "config.toml")

DEFAULT_CONFIG = """
broker_host = "127.0.0.1"
broker_port = 1883
data_dir = "~/sbc-guard-data"
rules_file = "rules.toml"

[ollama]
enabled = false
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "qwen3.5:0.8b"

[guard]
standby_minutes = 10
escalate_sev = "warn"
"""


def load_config() -> dict:
    raw = tomllib.loads(DEFAULT_CONFIG)
    if CFG_PATH.is_file():
        raw.update(tomllib.loads(CFG_PATH.read_text()))
    return raw


def load_rules(path: Path) -> list:
    if not path.is_file():
        # Sensible firmware-log defaults; override by editing rules.toml.
        return [
            {"name": "panic", "pattern": r"(panic|Guru Meditation|assert failed)", "sev": "error"},
            {"name": "err-log", "pattern": r"\b(E \(|ERROR|error:)", "sev": "error"},
            {"name": "warn-log", "pattern": r"\b(W \(|WARN|warning:)", "sev": "warn"},
            {"name": "rst", "pattern": r"(rst:|boot:|reboot)", "sev": "warn"},
        ]
    return tomllib.loads(path.read_text()).get("rule", [])


def compile_rules(rules: list) -> list:
    out = []
    for r in rules:
        try:
            out.append((r["name"], re.compile(r["pattern"]), r.get("sev", "warn")))
        except re.error as e:
            print(f"[rules] skipping {r.get('name')}: {e}", flush=True)
    return out


class Guard:
    def __init__(self, cfg: dict):
        self.cfg = cfg
        self.data_dir = Path(cfg.get("data_dir", "~/sbc-guard-data")).expanduser()
        self.data_dir.mkdir(parents=True, exist_ok=True)
        rules_path = Path(cfg.get("rules_file", "rules.toml"))
        if not rules_path.is_absolute():
            rules_path = CFG_PATH.parent / rules_path
        self.rules = compile_rules(load_rules(rules_path))
        self.o = cfg.get("ollama", {})
        self.g = cfg.get("guard", {})
        self.started = time.time()
        self.counters = {"frames": 0, "matches": 0, "llm_calls": 0, "llm_fail": 0}

    # ---- data plane ------------------------------------------------------
    def sink(self, node: str, frame: str):
        day = time.strftime("%Y%m%d")
        with (self.data_dir / f"events-{day}.jsonl").open("a", encoding="utf-8") as f:
            f.write(frame.replace("\n", " ") + "\n")
        self.counters["frames"] += 1
        _ = node  # node already inside frame

    # ---- pre-filter ------------------------------------------------------
    def match(self, text: str):
        for name, rx, sev in self.rules:
            m = rx.search(text)
            if m:
                return name, sev, m.group(0)[:120]
        return None

    # ---- small model (optional) ------------------------------------------
    def classify(self, text: str) -> dict | None:
        if not self.o.get("enabled"):
            return None
        prompt = (
            "Classify this embedded device log line. Reply ONLY one JSON object:\n"
            '{"sev":"debug|info|warn|error","summary":"<max 12 words>","category":"'
            '<wifi|power|sensor|mcu|other>"}\nLine: ' + text[:300]
        )
        for _attempt in range(2):  # one retry on invalid JSON
            self.counters["llm_calls"] += 1
            try:
                resp = requests.post(
                    self.o["url"],
                    json={
                        "model": self.o.get("model", "qwen3.5:0.8b"),
                        "messages": [{"role": "user", "content": prompt}],
                        "temperature": 0,
                        "max_tokens": 120,
                    },
                    timeout=90,
                )
                content = resp.json()["choices"][0]["message"]["content"]
                # strip thinking blocks / code fences defensively
                start, end = content.find("{"), content.rfind("}")
                obj = json.loads(content[start : end + 1])
                if {"sev", "summary"} <= set(obj) and obj["sev"] in ("debug", "info", "warn", "error"):
                    return obj
            except Exception:
                pass
            self.counters["llm_fail"] += 1
        return None

    def escalate(self, node: str, rule: str, sev: str, hit: str, full: str):
        llm = self.classify(full)
        sev = (llm or {}).get("sev", sev)
        summary = (llm or {}).get("summary") or hit
        alert = {
            "node": node,
            "ts": int(time.time()),
            "kind": "alert",
            "sev": sev,
            "rule": rule,
            "summary": summary,
            "payload": full[:400],
        }
        mqtt_client.publish(
            f"firment/device/{node}/alert", json.dumps(alert), qos=1
        )
        self.counters["matches"] += 1

    def heartbeat(self):
        status = {
            "service": "sbc-guard",
            "ts": int(time.time()),
            "uptime_s": int(time.time() - self.started),
            "standby_minutes": self.g.get("standby_minutes", 10),
            "escalate_sev": self.g.get("escalate_sev", "warn"),
            "rules": len(self.rules),
            "counters": self.counters,
        }
        mqtt_client.publish("firment/guard/status", json.dumps(status), retain=True)


def on_message(_c, _u, msg: mqtt.MQTTMessage):
    try:
        frame = msg.payload.decode("utf-8", "replace")
    except Exception:
        return
    node = "unknown"
    try:
        parsed = json.loads(frame)
        node = parsed.get("node", "unknown")
    except Exception:
        pass
    guard.sink(node, frame)

    # Never feed the model a firehose: only topic kinds worth watching.
    kind_topic = msg.topic.rsplit("/", 1)[-1]
    if kind_topic in ("state", "presence"):
        return
    hit = guard.match(frame)
    if hit and kind_topic != "alert":  # no alert-on-alert loops
        rule, sev, snippet = hit
        print(f"[hit] {msg.topic} rule={rule} sev={sev}: {snippet}", flush=True)
        guard.escalate(node, rule, sev, snippet, frame)


def on_connect(_c, _u, _f, rc, _p=None):
    print(f"[mqtt] connected rc={rc}", flush=True)
    mqtt_client.subscribe("firment/#", qos=1)


if __name__ == "__main__":
    cfg = load_config()
    guard = Guard(cfg)
    mqtt_client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2)
    mqtt_client.on_connect = on_connect
    mqtt_client.on_message = on_message
    mqtt_client.connect(cfg["broker_host"], int(cfg["broker_port"]), keepalive=30)
    beat = int(cfg.get("guard", {}).get("standby_minutes", 10)) * 60

    last_beat = 0.0
    mqtt_client.loop_start()
    print(
        f"[guard] up broker={cfg['broker_host']}:{cfg['broker_port']} "
        f"rules={len(guard.rules)} ollama={guard.o.get('enabled')} standby={beat // 60}min",
        flush=True,
    )
    while True:
        time.sleep(5)
        if time.time() - last_beat >= beat:
            guard.heartbeat()
            last_beat = time.time()
