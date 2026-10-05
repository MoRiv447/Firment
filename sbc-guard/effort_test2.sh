#!/bin/bash
# Ad-hoc probe script — NOT part of sbc-guard's deployment, and not needed to run the guard.
#
# It POSTs to OpenRouter to check how a model called `stealth/ox-alpha` treats
# `reasoning.enabled = false`. The conclusions are what ended up in this directory's
# `config.toml` notes (thinking models starve content tokens on a CPU-only box), which is why the
# script is kept rather than deleted.
#
# Three habits fixed here: the key was read from a fixed `/tmp/key.txt`, a path with no permission
# check and one any local user can plant; the response body went to a fixed `/tmp/rr.json`, which
# can be pre-created the same way; and neither was ever cleaned up. The key is now an exported
# variable and the body goes to a `mktemp` file that the trap removes.
: "${OPENROUTER_KEY:?export OPENROUTER_KEY=sk-... first — a readable file in /tmp is not a secret store}"
KEY="$OPENROUTER_KEY"

RR="$(mktemp)" || exit 1
trap 'rm -f "$RR"' EXIT

run() {
  NAME="$1"; EXTRA="$2"
  echo "=== $NAME ==="
  BODY="{\"model\":\"stealth/ox-alpha\",\"max_tokens\":3000$EXTRA,\"messages\":[{\"role\":\"user\",\"content\":\"Solve: if 3x+7=22, what is 5x? Answer briefly.\"}]}"
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o "$RR" -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  RR="$RR" python3 - <<'PY'
import json, os
try:
    r = json.load(open(os.environ['RR']))
    blocks = r.get('content', [])
    if not blocks and 'error' in r: print('  ERROR:', str(r['error'])[:120])
    tot = sum(len(b.get('thinking') or '') for b in blocks if b.get('type') in ('thinking','redacted_thinking'))
    txt = sum(len(b.get('text') or '') for b in blocks if b.get('type') == 'text')
    print(f'  thinking_chars={tot} text_chars={txt}')
except Exception as e:
    print('  err:', e)
PY
  sleep 4
}
run "enabled-false" ',"reasoning":{"enabled":false}'
run "omit-again"    ''
