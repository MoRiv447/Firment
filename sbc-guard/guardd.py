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
timeout_s = 60

[guard]
standby_minutes = 10
escalate_sev = "warn"
queue_max = 256
pairs_keep_days = 0
"""

# The rules used when rules.toml is absent OR unreadable. One list, not two: the fallback for a
# broken file and the fallback for a missing one have to be the same rules, or "delete the file"
# and "typo the file" would mean different things.
DEFAULT_RULES = [
    {"name": "panic", "pattern": r"(panic|Guru Meditation|assert failed)", "sev": "error"},
    {"name": "err-log", "pattern": r"\b(E \(|ERROR|error:)", "sev": "error"},
    {"name": "warn-log", "pattern": r"\b(W \(|WARN|warning:)", "sev": "warn"},
    {"name": "rst", "pattern": r"(rst:|boot:|reboot)", "sev": "warn"},
]

# Sections merged one level deep: a user [ollama] block omitting `enabled`
# must not wipe the rest of the defaults (raw dict.update clobbered tables).
SECTION_KEYS = ("ollama", "guard")


def load_config() -> dict:
    raw = tomllib.loads(DEFAULT_CONFIG)
    if not CFG_PATH.is_file():
        return raw
    try:
        user = tomllib.loads(CFG_PATH.read_text())
    except (tomllib.TOMLDecodeError, OSError) as e:
        # This used to propagate out of `main`: with `Restart=always` the unit then restart-looped
        # every 5 s (systemd's default start limit, 5 starts in 10 s, is not reached at that
        # cadence) and never reached `connect`/`subscribe` — so one stray quote in the config took
        # the DATA plane down, not just the alerts, while `journalctl` showed the same traceback
        # forever. Falling back to the defaults is the wrong repair here: the default broker is
        # 127.0.0.1, so a guard the operator pointed at another host would silently watch the wrong
        # one. Exit with a code the unit refuses to restart on instead (see
        # `RestartPreventExitStatus=2`), so systemd records a failure an operator can read.
        print(f"[config] {CFG_PATH} cannot be parsed: {e}", file=sys.stderr, flush=True)
        print(
            "[config] fix it, or remove it — a missing file uses the built-in defaults, "
            "a broken one would silently use the wrong broker",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(2)
    for key, value in user.items():
        if key in SECTION_KEYS and isinstance(value, dict) and isinstance(raw.get(key), dict):
            raw[key].update(value)
        else:
            raw[key] = value
    return raw


def load_rules(path: Path) -> list:
    if not path.is_file():
        return DEFAULT_RULES
    try:
        rules = tomllib.loads(path.read_text()).get("rule", [])
    except (tomllib.TOMLDecodeError, OSError) as e:
        # `compile_rules` below already refuses to let one bad RULE take the daemon down — that is
        # what its comment promises — but a bad FILE was not covered, and the failure it produced
        # was exactly the one that comment is about: the process died at startup, `Restart=always`
        # looped it, and nothing was subscribed. A stray quote in a hand-edited file marked the
        # whole board unwatched. Degrading to the built-in rules keeps the collector running.
        print(f"[rules] {path} cannot be parsed ({e}) — using the built-in defaults", flush=True)
        return DEFAULT_RULES
    if not rules:
        # Parses fine, declares nothing: the section is `[[rule]]` (singular) and a misspelled
        # table name yields an empty list, i.e. a guard that escalates nothing while looking
        # healthy. Loud, because that is silent deafness.
        print(
            f"[rules] {path} declares no [[rule]] entries — nothing will escalate "
            "(the table is `[[rule]]`, singular)",
            flush=True,
        )
    return rules


def compile_rules(rules: list) -> list:
    out = []
    for r in rules:
        # rules.toml is hand-edited: one rule with a missing key (or a `rule`
        # entry that is not a table at all) must skip THAT rule, not crash the
        # daemon on startup and leave the board unwatched.
        try:
            name = r["name"]  # raises first for every bad shape
            sev = r.get("sev", "warn")
            if sev not in ("debug", "info", "warn", "error"):
                print(
                    f"[rules] {name}: unknown sev {sev!r} — treated as error",
                    flush=True,
                )
            out.append((name, re.compile(r["pattern"]), sev))
        except (re.error, KeyError, TypeError) as e:
            label = r.get("name") if isinstance(r, dict) else repr(r)
            print(f"[rules] skipping {label or '<unnamed>'}: {e}", flush=True)
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
        # `llm_dropped` is in the heartbeat on purpose: shedding refinement under load is fine,
        # shedding it silently is not.
        self.counters = {
            "frames": 0,
            "matches": 0,
            "llm_calls": 0,
            "llm_fail": 0,
            "llm_dropped": 0,
        }
        self.escalate_sev = self.g.get("escalate_sev", "warn")
        # Bounded, because the single worker below can spend up to `2 × ollama.timeout_s` on one
        # item: a device stuck in a match storm enqueues faster than that drains, and an unbounded
        # queue turns the storm into unbounded memory — which the unit's `MemoryMax=256M` answers
        # with an OOM kill and `Restart=always` with a restart, i.e. a guard that dies exactly when
        # the device is misbehaving. Configurable; 256 items is a long backlog at the worst-case
        # drain rate and far more than an operator reads.
        self.queue_max = max(1, int(self.g.get("queue_max", 256)))
        self.work_queue: "queue.Queue" = queue.Queue(maxsize=self.queue_max)
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
        #
        # The timeout is a *drain rate*, not just a failure deadline: it multiplies by the two
        # attempts to give the worst case one queued item can hold the single worker, and that
        # product is what `queue_max` has to absorb. 180 s was sized for a thinking model; the
        # non-thinking classifier the config pins answers in seconds, so the default is 60.
        timeout_s = int(self.o.get("timeout_s", 60))
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
                    timeout=timeout_s,
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
        # One hit, one count — publish_alert also runs for the REVISED alert,
        # so the bump lives here rather than doubling every match.
        self.bump("matches")
        try:
            self.work_queue.put_nowait((node, rule, sev, hit, full))
        except queue.Full:
            # Never block this thread waiting for room — it is paho's network callback, and
            # blocking it is the same sin as classifying on it (the broker keepalive expires).
            # The RAW alert is already published above, so what a full queue costs is the LLM
            # refinement and its corpus sample for the newest hits; the counter puts that loss in
            # the heartbeat instead of in nobody's notice. Newest rather than oldest: the raw
            # alert for a recent hit is the one still on the operator's screen.
            self.bump("llm_dropped")

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

    def pairs_size(self) -> tuple:
        """`(files, bytes)` of the fine-tuning corpus — reported, not expired, by default."""
        files = total = 0
        for f in (self.data_dir / "pairs").glob("*.jsonl"):
            try:
                total += f.stat().st_size
                files += 1
            except OSError:
                pass
        return files, total

    def gc(self):
        """Expire what expires, and report what does not.

        The raw event sinks are an operational log: seven days is plenty, and the ceiling matters
        more than the history. `pairs/` is a training corpus — expiring it on a timer would throw
        away the thing it exists to accumulate — so it is kept until an operator says otherwise
        with `[guard] pairs_keep_days`, and its size rides the heartbeat either way.

        The comment here used to call the globbed set "daily sinks", plural, as though the whole
        directory were covered while `pairs/` grew without limit and nothing said so.
        """
        cutoff = time.time() - 7 * 86_400
        for old in self.data_dir.glob("events-*.jsonl"):
            try:
                if old.stat().st_mtime < cutoff:
                    old.unlink()
                    print(f"[gc] removed {old.name}", flush=True)
            except OSError:
                pass
        keep_days = int(self.g.get("pairs_keep_days", 0))
        if keep_days <= 0:
            return
        pairs_cutoff = time.time() - keep_days * 86_400
        for old in (self.data_dir / "pairs").glob("*.jsonl"):
            try:
                if old.stat().st_mtime < pairs_cutoff:
                    old.unlink()
                    print(f"[gc] removed pairs/{old.name}", flush=True)
            except OSError:
                pass

    def heartbeat(self):
        pairs_files, pairs_bytes = self.pairs_size()
        status = {
            "service": "sbc-guard",
            "online": True,
            "ts": int(time.time()),
            "uptime_s": int(time.time() - self.started),
            "standby_minutes": self.g.get("standby_minutes", 10),
            "escalate_sev": self.escalate_sev,
            "rules": len(self.rules),
            # The backlog and its ceiling, so `llm_dropped` is actionable: a climbing count with a
            # full queue means refinement is being shed, and that is a knob, not a mystery.
            "queue_depth": self.work_queue.qsize(),
            "queue_max": self.queue_max,
            # The corpus is kept by default, so its growth has to be visible somewhere — see
            # `gc`. A number in every heartbeat is enough to notice it climbing.
            "pairs_files": pairs_files,
            "pairs_bytes": pairs_bytes,
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
            # GC: the raw event sinks. `pairs/` is reported in the heartbeat rather than expired
            # by default — see `Guard.gc` for why the two sets are held differently.
            guard.gc()
