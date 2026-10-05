#!/usr/bin/env python3
"""Smoke test for guardd.py's startup failure paths — the two repaired on 2026-10-05.

`guardd.py` imports paho-mqtt and requests, which an SBC has and a build box may not, so this
supplies stub modules in a temp tree and runs the daemon as a real subprocess. Standard library
only; run it with `python3 test_guardd.py` from this directory.

What each case is for:

  A  rules.toml does not parse  -> warn, fall back to the built-in rules, KEEP COLLECTING.
     The old behaviour was an uncaught TOMLDecodeError: the process died at startup, the unit's
     `Restart=always` looped it every 5 s, and nothing was subscribed — so one stray quote took
     the data plane down, not just the alerts.
  B  config.toml does not parse -> exit 2 with the reason, which the unit's
     `RestartPreventExitStatus=2` turns into a recorded failure instead of a restart loop.
     Falling back to the defaults would silently point the guard at the wrong broker.
  C  both files fine -> still reaches `[guard] up`; the repairs did not break the happy path.
  D  a misspelled `[[rules]]` table -> says nothing will escalate. Parses fine, declares nothing,
     and an empty rule list is a guard that looks healthy and escalates nothing.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

GUARDD = Path(__file__).resolve().parent / "guardd.py"

PAHO_CLIENT = '''
class CallbackAPIVersion:
    VERSION2 = 2


class MQTTMessage:
    def __init__(self, topic="", payload=b""):
        self.topic = topic
        self.payload = payload


class Client:
    def __init__(self, *a, **k):
        self.on_connect = None
        self.on_message = None

    def will_set(self, *a, **k):
        pass

    def connect(self, *a, **k):
        pass

    def loop_start(self):
        pass

    def subscribe(self, *a, **k):
        pass

    def publish(self, *a, **k):
        return (0, 1)
'''

REQUESTS_STUB = '''
def post(*a, **k):
    raise RuntimeError("stub: no ollama here")
'''

GOOD_CONFIG = '''
broker_host = "127.0.0.1"
broker_port = 1883
data_dir = "{data}"
rules_file = "rules.toml"

[ollama]
enabled = false

[guard]
standby_minutes = 10
escalate_sev = "warn"
'''

GOOD_RULES = '''
[[rule]]
name = "panic"
pattern = '(panic|assert failed)'
sev = "error"
'''


def build_tree(config_text: str, rules_text: str) -> Path:
    root = Path(tempfile.mkdtemp(prefix="guardd-smoke-"))
    (root / "paho" / "mqtt").mkdir(parents=True)
    (root / "paho" / "__init__.py").write_text("from . import mqtt\n")
    (root / "paho" / "mqtt" / "__init__.py").write_text("from . import client\n")
    (root / "paho" / "mqtt" / "client.py").write_text(PAHO_CLIENT)
    (root / "requests.py").write_text(REQUESTS_STUB)
    shutil.copy(GUARDD, root / "guardd.py")
    # `as_posix()` because a TOML basic string reads a backslash as an escape: a Windows temp
    # path spliced in raw is what the first version of this test actually measured.
    (root / "config.toml").write_text(config_text.format(data=(root / "data").as_posix()))
    (root / "rules.toml").write_text(rules_text)
    return root


def run(root: Path, seconds: int = 4):
    """Return (exit code, stdout, stderr). 124 means it was still up at the deadline."""
    env = dict(os.environ)
    env["PYTHONPATH"] = str(root)
    try:
        p = subprocess.run(
            [sys.executable, "guardd.py"],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            timeout=seconds,
        )
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        return 124, e.stdout or "", e.stderr or ""


def case(name: str, config_text: str, rules_text: str, want, seconds: int = 4) -> bool:
    root = build_tree(config_text, rules_text)
    try:
        code, out, err = run(root, seconds)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    ok = want(code, out, err)
    print(f"{'PASS' if ok else 'FAIL'}  {name}  (exit={code})")
    for line in (out + err).strip().splitlines():
        print(f"        | {line}")
    return ok


def main() -> int:
    results = [
        case(
            "A rules.toml broken -> warn + built-in rules + stays up",
            GOOD_CONFIG,
            "[[rule]\nname = broken\n",
            lambda c, o, e: c == 124 and "using the built-in defaults" in o,
        ),
        case(
            "B config.toml broken -> exit 2 + reason on stderr",
            "broker_host = \n",
            GOOD_RULES,
            lambda c, o, e: c == 2 and "cannot be parsed" in e,
            seconds=20,
        ),
        case(
            "C both fine -> reaches [guard] up",
            GOOD_CONFIG,
            GOOD_RULES,
            lambda c, o, e: c == 124 and "[guard] up" in o,
        ),
        case(
            "D misspelled [[rules]] -> says nothing will escalate",
            GOOD_CONFIG,
            "[[rules]]\nname = 'x'\npattern = 'y'\n",
            lambda c, o, e: c == 124 and "declares no [[rule]] entries" in o,
        ),
    ]
    print()
    print(f"{sum(results)}/{len(results)} cases passed")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
