#!/bin/bash
KEY="$1"
run() {
  NAME="$1"; EXTRA="$2"
  echo "=== $NAME ==="
  BODY="{\"model\":\"stealth/ox-alpha\",\"max_tokens\":3000$EXTRA,\"messages\":[{\"role\":\"user\",\"content\":\"Solve: if 3x+7=22, what is 5x? Show reasoning then answer.\"}]}"
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o /tmp/rr.json -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  python3 - <<'PY'
import json
try:
    r = json.load(open('/tmp/rr.json'))
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
