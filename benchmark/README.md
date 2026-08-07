# 终端编码 Agent 工程质量横向评测方案

**被测对象**：opencode、Claude Code、oh-my-pi (omp)
**统一模型**：DeepSeek（三者接入完全相同的模型、端点与参数，模型能力是控制变量，差异只来自 agent 工程：系统提示、工具设计、编辑格式、上下文管理、错误恢复）

## 1. 评测原则（控制变量）

| 变量 | 控制方式 |
|---|---|
| 模型 | 同一 DeepSeek 模型（如 `deepseek-chat` / `deepseek-reasoner`，选定一个全程不换）、同一 API key、同一 base_url |
| 采样参数 | 全部使用各 agent 默认值；若可配置则显式固定 temperature=0 |
| 硬件/网络 | 同一台机器、同一时间段跑完，避免 API 波动期 |
| 工作区 | 每个用例每次运行都从 fixtures 的干净副本开始（git 初始化，便于 diff 统计） |
| 调用方式 | 统一用各 CLI 的 one-shot 模式：`opencode run "<prompt>"`、`claude -p "<prompt>"`、`omp -p "<prompt>"` |
| 轮次 | 每个用例每个 agent 跑 **3 次**，记录最好/最差/中位，避免单次运气 |
| 权限 | 三者给予等价的自动批准级别（自动允许文件编辑和 bash），否则交互确认会污染计时 |

## 2. 目录结构

```
agent-benchmark/
├── README.md            # 本文件：方法论与运行协议
├── 测试用例.md          # 24 个用例：提示词原文、预期、判分检查点
├── 评分标准.md          # 维度、权重、0-5 锚定量规、汇总公式
├── 评分表.csv           # 打分模板，Excel 直接打开
└── fixtures/
    └── todo_app/        # 测试夹具：含 3 个预埋 bug 的 Python 小项目
        ├── todo.py          # BUG-1 日期解析 off-by-one；BUG-2 complete 索引基准错；BUG-3 序列化键名不一致丢状态
        ├── test_todo.py     # 参考测试（初始 3 个失败，对应 3 个 bug）
        └── FEATURE_BRIEF.md # 新功能需求（优先级排序，供 F 类用例）
```

## 3. 运行协议（每个用例）

```bash
# 1. 准备干净工作区
rm -rf run && cp -r fixtures/todo_app run && cd run && git init -q && git add -A && git commit -qm init

# 2. 计时运行（以 omp 为例，其余同理换命令）
time omp -p "<测试用例.md 中的提示词原文>"

# 3. 采集数据
git diff --stat            # 改动文件数/行数（检查附带损伤）
python -m pytest test_todo.py -q   # 功能正确性（B/F/R 类）
# 记录：总轮数(turns)、token 用量（各 agent 的 cost/usage 输出）、墙钟时间、是否需人工介入
```

## 4. 结果汇总

- 单用例得分 = 检查点达成率 × 5，按《评分标准.md》锚定量规取档
- 3 次运行取中位数作为该用例得分
- 总分 = Σ(维度均分 × 权重)，权重见评分标准
- 除质量分外，单独报告**效率榜**（token / 轮数 / 耗时），不与质量分混合

## 5. 已知偏差与声明

- 各 agent 对 DeepSeek 的适配深度不同（如 omp 的 hash-anchored 编辑、Claude Code 的系统提示为 Anthropic 模型调优），这本身就是"工程质量"的一部分，不做拉平处理。
- one-shot 模式屏蔽了 TUI 交互体验差异；如需评测交互体验，另开一组人工主观评分，不计入总分。
- 若某 agent 不支持某能力（如计划模式差异），按"无法完成"计 0 分，并备注。
