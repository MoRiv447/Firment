#!/bin/bash
# run_agent.sh <cc|oc|omp|codex|firm> — 顺序跑完 19 个用例，采集日志/diff/pytest/耗时
BASE="/d/OldStudy66/ai test"
# codex 的 npm shim 路径损坏，用 AppData 下的真实 exe（版本哈希目录，用 glob 动态定位）
CODEX_BIN=$(ls "C:/Users/18978/AppData/Local/OpenAI/Codex/bin"/*/codex.exe 2>/dev/null | head -1)
AGENT="$1"
CASES="A2 A3 B1 B2 B3 B4 C1 C2 F1 F2 R1 R2 T1 T2 L1 L2 S1 S2"  # A1 已在冒烟测试完成，单独补跑见下
CASES="A1 $CASES"

for CASE in $CASES; do
  WS="$BASE/$AGENT/$CASE"
  bash "$BASE/bench/prep.sh" "$AGENT" "$CASE" >/dev/null 2>&1
  PROMPT="$(cat "$BASE/bench/prompts/$CASE.txt")"
  cd "$WS" || exit 1
  START=$(date +%s)
  case "$AGENT" in
    cc)
      MAX_THINKING_TOKENS=32000 claude -p "$PROMPT" --dangerously-skip-permissions > session.log 2>&1
      ;;
    oc)
      opencode run "$PROMPT" > session.log 2>&1
      ;;
    omp)
      ~/.bun/bin/omp -p --auto-approve --thinking max --model deepseek/deepseek-v4-flash --max-time 15m "$PROMPT" > session.log 2>&1
      ;;
    codex)
      # 非交互 exec + 跳过所有审批/沙箱；显式锁模型=deepseek-v4-flash、推理=max
      "$CODEX_BIN" exec --dangerously-bypass-approvals-and-sandbox \
        -c 'model="deepseek-v4-flash"' -c 'model_provider="deepseek"' -c 'model_reasoning_effort="max"' \
        "$PROMPT" > session.log 2>&1
      ;;
    firm)
      # 用 release 编译好的 exe（绝对路径，避免 PATH 未装 firm 的情况）；
      # -y 自动批准写/编辑/shell；显式锁模型=deepseek-v4-flash、推理=max，与另四家一致。
      # 认证走 %APPDATA%/firment/auth.json，不依赖 shell 的 DEEPSEEK_API_KEY。
      "D:/OldStudy66/Firment/target/release/firm.exe" -y -p "$PROMPT" \
        --thinking max --model deepseek-v4-flash --provider default > session.log 2>&1
      ;;
  esac
  END=$(date +%s)
  echo $((END - START)) > time.txt
  git diff > diff.patch 2>/dev/null
  git status --porcelain > status.txt 2>/dev/null
  if [ "$CASE" = "T2" ]; then
    .venv/Scripts/python -m pytest test_todo.py -q > pytest.txt 2>&1
  else
    python -m pytest test_todo.py -q > pytest.txt 2>&1
  fi
  echo "$AGENT $CASE done in $((END - START))s"
done
echo "=== $AGENT SUITE COMPLETE ==="
