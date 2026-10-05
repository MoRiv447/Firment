#!/bin/bash
# Ad-hoc probe script — NOT part of sbc-guard's deployment, and not needed to run the guard.
#
# It POSTs to OpenRouter to compare how a model called `stealth/ox-alpha` treats the `reasoning`
# effort levels. The conclusions are what ended up in this directory's `config.toml` notes
# (thinking models starve content tokens on a CPU-only box), which is why the script is kept
# rather than deleted.
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
  NAME="$1"; EXTRA="$2"
  echo "=== $NAME ==="
  BODY="{\"model\":\"stealth/ox-alpha\",\"max_tokens\":3000$EXTRA,\"messages\":[{\"role\":\"user\",\"content\":\"Solve: if 3x+7=22, what is 5x? Show reasoning then answer.\"}]}"
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o "$RR" -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  RR="$RR" python3 - <<'PY'
import json, os
try:
    r = json.load(open(os.environ['RR']))
    blocks = r.get('content', [])
    if not blocks and 'error' in r: print('  ERROR:', str(r['error'])[:120])
    tot_think = sum(len(b.get('thinking') or '') for b in blocks if b.get('type') in ('thinking','redacted_thinking'))
    tot_text = sum(len(b.get('text') or '') for b in blocks if b.get('type') == 'text')
    print(f'  thinking_chars={tot_think} text_chars={tot_text}')
    for b in blocks:
        if b.get('type') in ('thinking','redacted_thinking'):
            print('  think head:', (b.get('thinking') or b.get('data') or '')[:90])
except Exception as e:
    print('  err:', e)
PY
  sleep 3
}
run "omit(off)"      ''
run "effort-low"     ',"reasoning":{"effort":"low"}'
run "effort-medium"  ',"reasoning":{"effort":"medium"}'
run "effort-high"    ',"reasoning":{"effort":"high"}'
