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
import queue
import re
import sys
import threading
import time
from pathlib import Path
from typing import Optional

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
model = "qwen2.5:0.5b"

[guard]
standby_minutes = 10
escalate_sev = "warn"
"""

# Sections merged one level deep: a user [ollama] block omitting `enabled`
# must not wipe the rest of the defaults (raw dict.update clobbered tables).
SECTION_KEYS = ("ollama", "guard")


def load_config() -> dict:
    raw = tomllib.loads(DEFAULT_CONFIG)
    if CFG_PATH.is_file():
        user = tomllib.loads(CFG_PATH.read_text())
        for key, value in user.items():
            if key in SECTION_KEYS and isinstance(value, dict) and isinstance(raw.get(key), dict):
                raw[key].update(value)
            else:
                raw[key] = value
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
            sev = r.get("sev", "warn")
            if sev not in ("debug", "info", "warn", "error"):
                print(
                    f"[rules] {r['name']}: unknown sev {sev!r} — treated as error",
                    flush=True,
                )
            out.append((r["name"], re.compile(r["pattern"]), sev))
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
        self.counters_lock = threading.Lock()
        self.counters = {"frames": 0, "matches": 0, "llm_calls": 0, "llm_fail": 0}
        self.escalate_sev = self.g.get("escalate_sev", "warn")
        self.work_queue: "queue.Queue" = queue.Queue()
        threading.Thread(target=self._worker, daemon=True).start()

    _SEV_RANK = {"debug": 0, "info": 1, "warn": 2, "error": 3}

    def rank(self, sev: str) -> int:
        # Unknown sev strings rank as MOST severe: a custom rules.toml sev
        # like "critical" must escalate, never silently sink to disk.
        return self._SEV_RANK.get(sev, 3)

    def bump(self, key: str, n: int = 1):
        # counters are touched from the callback thread, the worker thread
        # and the heartbeat loop — plain += loses increments.
        with self.counters_lock:
            self.counters[key] += n

    def snapshot(self) -> dict:
        with self.counters_lock:
            return dict(self.counters)

    # ---- data plane ------------------------------------------------------
    def sink(self, node: str, frame: str):
        day = time.strftime("%Y%m%d")
        with (self.data_dir / f"events-{day}.jsonl").open("a", encoding="utf-8") as f:
            f.write(frame.replace("\n", " ") + "\n")
        self.bump("frames")
        _ = node  # node already inside frame

    # ---- pre-filter ------------------------------------------------------
    def match(self, text: str):
        for name, rx, sev in self.rules:
            m = rx.search(text)
            if m:
                return name, sev, m.group(0)[:120]
        return None

    # ---- small model (optional) ------------------------------------------
    def classify(self, text: str) -> Optional[dict]:
        if not self.o.get("enabled"):
            return None
        prompt = (
            "Classify this embedded device log line. Reply ONLY one JSON object:\n"
            '{"sev":"debug|info|warn|error","summary":"<max 12 words>","category":"'
            '<wifi|power|sensor|mcu|other>"}\nLine: ' + text[:300]
        )
        # qwen3.5 is a THINKING model: its reasoning consumes output tokens
        # before any content appears (P0 notes). Budget generously or
        # content comes back empty every time.
        for _attempt in range(2):  # one retry on invalid JSON
            self.bump("llm_calls")
            try:
                resp = requests.post(
                    self.o["url"],
                    json={
                        "model": self.o.get("model", "qwen2.5:0.5b"),
                        "messages": [{"role": "user", "content": prompt}],
                        "temperature": 0,
                        "max_tokens": 800,
                    },
                    timeout=180,
                )
                msg = resp.json()["choices"][0]["message"]
                content = msg.get("content") or ""
                # Strip a <think>...</think> block if the template inlined it.
                content = re.sub(r"<think>.*?</think>", "", content, flags=re.S)
                start, end = content.find("{"), content.rfind("}")
                obj = json.loads(content[start : end + 1])
                if {"sev", "summary"} <= set(obj) and obj["sev"] in ("debug", "info", "warn", "error"):
                    return obj
                print(f"[llm] attempt {_attempt + 1}: schema miss: {content[:120]!r}", flush=True)
            except Exception as e:
                print(f"[llm] attempt {_attempt + 1} failed: {e}", flush=True)
            self.bump("llm_fail")
        return None

    def enqueue_escalate(self, node: str, rule: str, sev: str, hit: str, full: str):
        """Two-phase publish: the RAW alert goes out immediately (latency
        beats polish), then the worker classifies and publishes a REVISED
        alert. Classification never runs on the paho callback thread — the
        broker keepalive would expire mid-call."""
        self.publish_alert(node, rule, sev, hit, full, revised=False)
        self.work_queue.put((node, rule, sev, hit, full))

    def _worker(self):
        while True:
            node, rule, sev, hit, full = self.work_queue.get()
            try:
                llm = self.classify(full)
                # The RULE severity is authoritative; the LLM only refines.
                # Record every classification for the fine-tuning corpus
                # BEFORE publishing so pairs/ captures what actually shipped.
                self.record_pair(node, rule, sev, hit, full, llm)
                if llm:
                    self.publish_alert(
                        node,
                        rule,
                        llm.get("sev", sev),
                        llm.get("summary") or hit,
                        full,
                        revised=True,
                    )
            except Exception as e:
                print(f"[worker] classify failed: {e}", flush=True)
            finally:
                self.work_queue.task_done()

    def record_pair(self, node, rule, sev, hit, full, llm):
        """Append one fine-tuning sample to data_dir/pairs/<YYYYMMDD>.jsonl.

        Shape: rule identity + authoritative rule_sev, the raw line, and the
        small-model's opinion (llm_*) when it produced one. Training later
        weighs these against rule_sev; review scripts can diff llm_sev vs
        rule_sev to mine corrections.
        """
        try:
            pdir = self.data_dir / "pairs"
            pdir.mkdir(parents=True, exist_ok=True)
            rec = {
                "ts": int(time.time()),
                "node": node,
                "rule": rule,
                "rule_sev": sev,
                "hit": hit,
                "line": full[:400],
                "published_sev": (llm or {}).get("sev", sev) if llm else sev,
            }
            if llm:
                rec["llm_sev"] = llm.get("sev")
                rec["llm_summary"] = (llm.get("summary") or "")[:120]
                rec["llm_category"] = llm.get("category")
            with open(
                pdir / f"{time.strftime('%Y%m%d')}.jsonl", "a", encoding="utf-8"
            ) as f:
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        except Exception as e:
            # Corpus failure must never break alerting.
            print(f"[pairs] record failed: {e}", flush=True)

    def publish_alert(
        self, node: str, rule: str, sev: str, summary: str, full: str, revised: bool
    ):
        alert = {
            "node": node,
            "ts": int(time.time()),
            "kind": "alert",
            "sev": sev,
            "rule": rule,
            "summary": summary,
            "payload": full[:400],
        }
        if revised:
            alert["revised"] = True
        mqtt_client.publish(f"firment/device/{node}/alert", json.dumps(alert), qos=1)
        self.bump("matches")

    def heartbeat(self):
        status = {
            "service": "sbc-guard",
            "online": True,
            "ts": int(time.time()),
            "uptime_s": int(time.time() - self.started),
            "standby_minutes": self.g.get("standby_minutes", 10),
            "escalate_sev": self.escalate_sev,
            "rules": len(self.rules),
            "counters": self.snapshot(),
        }
        mqtt_client.publish("firment/guard/status", json.dumps(status), retain=True)


def on_message(_c, _u, msg: mqtt.MQTTMessage):
    # One bad frame (or a failing sink) must NEVER kill the paho network
    # thread: the process would keep heartbeating while deaf to all traffic,
    # and systemd would never restart it.
    try:
        handle_message(msg)
    except Exception as e:
        print(f"[on_message] dropped frame due to error: {e}", flush=True)


def handle_message(msg: mqtt.MQTTMessage):
    try:
        # Strip CR/LF: file-based publishers (-f) and serial bridges often
        # append newlines that would otherwise leak into stored/classified
        # payloads.
        frame = msg.payload.decode("utf-8", "replace").strip()
    except Exception:
        return
    if not frame:
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
    # Bound regex work: frames are broker-capped (~256KB) and a pathological
    # user pattern could otherwise hang the network thread.
    hit = guard.match(frame[:4096])
    if hit and kind_topic != "alert":  # no alert-on-alert loops
        rule, sev, snippet = hit
        # escalate_sev gate: the pre-filter catches everything at/above the
        # configured floor; quieter hits are sunk to disk only.
        if guard.rank(sev) < guard.rank(guard.escalate_sev):
            return
        print(f"[hit] {msg.topic} rule={rule} sev={sev}: {snippet}", flush=True)
        guard.enqueue_escalate(node, rule, sev, snippet, frame)


def on_connect(_c, _u, _f, rc, _p=None):
    print(f"[mqtt] connected rc={rc}", flush=True)
    mqtt_client.subscribe("firment/#", qos=1)


if __name__ == "__main__":
    cfg = load_config()
    guard = Guard(cfg)
    # clean_session=False + stable client id: a broker with persistence
    # queues QoS1 frames across disconnects — "never drop" extends to
    # outages, per the docstring. paho requires an explicit id for that.
    mqtt_client = mqtt.Client(
        mqtt.CallbackAPIVersion.VERSION2, client_id="sbc-guard", clean_session=False
    )
    mqtt_client.on_connect = on_connect
    mqtt_client.on_message = on_message
    # LWT: a crashed daemon flips the retained status to online=false, so
    # consumers can tell "dead since ts" from "alive".
    mqtt_client.will_set(
        "firment/guard/status",
        json.dumps({"service": "sbc-guard", "online": False}),
        qos=1,
        retain=True,
    )
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
            # GC: daily sinks older than 7 days are deleted on the heartbeat.
            cutoff = time.time() - 7 * 86_400
            for old in guard.data_dir.glob("events-*.jsonl"):
                try:
                    if old.stat().st_mtime < cutoff:
                        old.unlink()
                        print(f"[gc] removed {old.name}", flush=True)
                except OSError:
                    pass
