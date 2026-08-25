#!/bin/bash
KEY="$1"
run() {
  NAME="$1"; BODY="$2"
  echo "=== $NAME ==="
  curl -s -m 120 https://openrouter.ai/api/v1/messages -H "x-api-key: $KEY" -H 'content-type: application/json' -d "$BODY" -o /tmp/rr.json -w 'HTTP=%{http_code} TIME=%{time_total}\n'
  python3 - <<'PY'
import json
try:
    r = json.load(open('/tmp/rr.json'))
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
