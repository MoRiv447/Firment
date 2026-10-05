#!/bin/bash
# Ad-hoc probe script — NOT part of sbc-guard's deployment, and not needed to run the guard.
#
# It POSTs to OpenRouter to compare how a model called `stealth/ox-alpha` treats the various
# thinking / reasoning parameters. The conclusions are what ended up in this directory's
# `config.toml` notes (thinking models starve content tokens on a CPU-only box), which is why the
# script is kept rather than deleted.
#
# Two habits fixed here: the key used to arrive as `$1`, so it landed in the shell history and in
# `ps` output for every user on the machine; and the response body went to a fixed `/tmp/rr.json`,
# a path any local user can pre-create. The key is now an exported variable and the body goes to a
# `mktemp` file.
: "${OPENROUTER_KEY:?export OPENROUTER_KEY=sk-... first — a key on the command line lands in history and in ps}"
KEY="$OPENROUTER_KEY"

RR="$(mktemp)" || exit 1
trap 'rm -f "$RR"' EXIT

run() {
  NAME="$1"; BODY="$2"
  echo "=== $NAME ==="
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o "$RR" -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  RR="$RR" python3 - <<'PY'
import json, os
try:
    r = json.load(open(os.environ['RR']))
    blocks = r.get('content', [])
    if not blocks and 'error' in r: print('  ERROR:', str(r['error'])[:150])
    for b in blocks:
        t = b.get('type')
        raw = b.get('thinking') or b.get('text') or ''
        print(f"  type={t} len={len(raw)} head={raw[:70]!r}")
    print('  stop:', r.get('stop_reason'), ' reasoning_details:', bool(r.get('reasoning_details')))
except Exception as e:
    print('  err:', e)
PY
  sleep 3
}
run "off-omit"      '{"model":"stealth/ox-alpha","max_tokens":3000,"messages":[{"role":"user","content":"What is 15*23? Answer briefly."}]}'
run "think-only"    '{"model":"stealth/ox-alpha","max_tokens":3000,"thinking":{"type":"enabled","budget_tokens":2048},"messages":[{"role":"user","content":"What is 15*23? Answer briefly."}]}'
run "reason-only"   '{"model":"stealth/ox-alpha","max_tokens":3000,"reasoning":{"effort":"high"},"messages":[{"role":"user","content":"What is 15*23? Answer briefly."}]}'
run "reason-maxtok" '{"model":"stealth/ox-alpha","max_tokens":3000,"reasoning":{"max_tokens":2048},"messages":[{"role":"user","content":"What is 15*23? Answer briefly."}]}'
