#!/bin/bash
KEY=$(cat /tmp/key.txt)
run() {
  NAME="$1"; EXTRA="$2"
  echo "=== $NAME ==="
  BODY="{\"model\":\"stealth/ox-alpha\",\"max_tokens\":3000$EXTRA,\"messages\":[{\"role\":\"user\",\"content\":\"Solve: if 3x+7=22, what is 5x? Answer briefly.\"}]}"
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o /tmp/rr.json -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  python3 - <<'PY'
import json
try:
    r = json.load(open('/tmp/rr.json'))
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
