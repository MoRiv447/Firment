# GUI 工作台设计 —— 面向长期项目的项目中枢（v1 草案）

> 状态：设计定稿，分期实施（W1 → W4）。
> 配套文档：`docs/sbc-agent.md`（SBC 小模型数据桥；守卫与遥测的事件总线在 §5 汇合）。

## 0. 定位与三原则

工作台取代当前 GUI 的"团队"占位菜单，成为**长期嵌入式项目的项目中枢**。
它首先为单人服务，多人协作是叠加层。

三原则：

1. **单人优先。** 每个阶段都必须对独立开发者产生日常价值；多人功能是
   叠加而非前提，避免"协作鬼城"。
2. **建在 git 之上，不重造版本管理。** 支线可绑定 git 分支；跨作用域的
   修改请求本质是轻量 PR（快照比对 + 应用/拒绝）；冲突交给 git。
3. **仓库即状态。** 工作台的全部状态存放在仓库内 `.firment/workbench.toml`
   与 JSONL 文件中：单人零依赖开箱即用；团队靠 git push/pull 天然同步；
   CLI/TUI 读同一份状态——三端共享的是仓库，不是某个服务器。

## 1. 功能全景（六域分级）

标注：✓ = 用户原始需求；★ = 本设计补充。期数对应 §6。

### A. 项目结构与对话

| 功能 | 期 | 说明 |
|---|---|---|
| ✓ 主线对话 | W1 | 项目长期上下文，跨天续用；一个项目一条主线 |
| ✓ 支线对话 | W1 | 会话树子节点：实验、子任务、调试；可选绑定 git 分支 |
| ★ 会话检查点 | W1 | 代码+对话状态快照；支线失败一键回到检查点 |
| ★ 项目模板 | W1 | 新建项目生成 `.firment/` 骨架（workbench.toml、hil.toml 模板、AGENTS.md）|
| ★ 会话搜索/归档 | W2 | 大项目的旧支线折叠不删 |

### B. 状态与追踪

| 功能 | 期 | 说明 |
|---|---|---|
| ✓ 仓库状态卡 | W1 | 分支/脏文件数/最近提交（复用 TUI 同款逻辑）|
| ✓ 项目现状 | W1 | todo 任务卡片化 |
| ✓ 变更时间线 | W1 | journal/ledger 渲染，每条带 undo 入口 |
| ★ ELF 预算卡 | W1 | flash/RAM 占用进度条 vs 基线（`elf_analyze` 数据现成）；接近阈值变色并与 ELF 门禁阈值联动——**嵌入式身份标识** |
| ★ 验证状态徽章 | W1 | 最近 build/verify/hil 结果；存在未验证变更时高亮（数据来自门禁计数）|
| ★ 烧录历史 | W3 | 何日向哪块板烧了哪个镜像（hil replay JSONL 串起）|
| ★ 硬件清单 | W3 | 探针/串口/芯片列表 |

### C. 快捷指令

| 功能 | 期 | 说明 |
|---|---|---|
| ✓ 内置指令按钮 | W1 | 构建/烧录/监控/验证/HIL 一键化（现有斜杠命令按钮化）|
| ★ 自定义参数化指令 | W1 | 存 workbench.toml；名字→提示词或步骤序列，支持 `{port}` 占位 |

### D. 知识管理

| 功能 | 期 | 说明 |
|---|---|---|
| ★ 项目知识库入口 | W2 | AGENTS.md 编辑 + `.firment/` 项目私有 cheatsheets（kb 已支持项目级 vendor-index 合并，纯缺 UI）|
| ★ 决策记录 ADR-lite | W2 | 选型/引脚分配/协议决策存为可检索条目；新支线自动注入相关决策 |
| ★★ 引脚/资源分配表 | W2 | PA5=LED、PB7=中断……agent 动外设前自动查冲突，与 `periph_init` 联动；存 workbench.toml `[pinmap]` |

### E. 审查流（多人附属层）

| 功能 | 期 | 说明 |
|---|---|---|
| ✓ 作用域 + ChangeRequest | W2 | 出界变更打包发作用域主人审批（详见 §4）|
| ★ CR 审查检查单 | W2 | 测过没 / 文档更没更 / 引脚表更新没 |
| ★ 通知中心 | W3 | CR 到达、守卫升级、构建失败汇总一处 |

### F. 小模型守卫（与 sbc-agent.md 合流）

| 功能 | 期 | 说明 |
|---|---|---|
| ✓ 待机时长 / 实时上报配置 | W3 | 守卫节奏可调 |
| ★ 守卫规则可视化 | W3 | 对应采集器预过滤器的关键词/阈值表 |
| ★ 守卫统计 | W3 | 触发次数/误报率（也是微调数据的健康度指标）|

## 2. 数据模型：`.firment/workbench.toml`

```toml
[project]
name = "fw-thermostat"
created_at = 1755850000

[workbench]
mainline_session = "e5bd87a2-057c-4a3e-a87b-7cb5bf6b3335"
guard = { enabled = false, standby_minutes = 30, escalate_sev = "warn" }

# 会话树：主线之外的每个节点一条 [branch.<session-id 前 8 位>]
[branch.a1b2c3d4]
parent = "mainline"            # 或另一支线 id，构成树
title = "传感器漂移排查"
git_branch = "exp/drift-hunt"  # 可选；绑定后 agent 的提交落此分支
status = "open"                # open | merged | archived
created_at = 1755851000

# 作用域：成员 → 路径 glob 列表（W2；单人模式只有 owner 一项，天然成立）
[scope.owner]
member = "alice"
paths = ["**"]                 # 默认全域

[scope.bob]
member = "bob"
paths = ["src/drivers/**", "hal/**"]

# 快捷指令：二选一（prompt 或 steps），{port} 类占位在触发时替换
[quickcmd.build]
steps = ["build"]

[quickcmd.flash-usb0]
steps = ["flash", "monitor"]
vars = { port = "COM14" }

# 引脚/资源分配表（W2）：periph_init 生成骨架前先查冲突
[pinmap.PA5]
func = "LED status"
owner = "alice"

[pinmap.PB7]
func = "sensor IRQ (exti)"
owner = "alice"

# 决策记录 ADR-lite（W2）
[[decision]]
title = "传感器总线选 CAN 而非 RS485"
body = "节点数可能扩到 16；已有 S31 自带 CAN FD……"
date = "2026-08-22"
```

约定：

- **文件即真相**：GUI 写、CLI 读；损坏容忍策略与会话 JSONL 相同（跳过坏行，
  关键段缺失时报错而非猜测）。
- `branch.*.parent` 构成会话树；树渲染即工作台的"项目结构"视图。

## 3. 会话树模型

`SessionStore` 的 MetaLine 扩展两个可选字段（serde default 向后兼容旧
JSONL，无需迁移）：

```rust
#[serde(default)]
pub parent_session: Option<String>,  // None = 主线候选/独立会话
#[serde(default)]
pub kind: SessionKind,               // Main | Branch（default Main）
```

- 新建支线 = 复制父会话元数据 + 设 parent/kind（消息不继承：支线是新的
  上下文，需要历史由用户显式 `/pin` 或引用检查点）；
- CLI/TUI/GUI 三端从同一 store 读取，树形展示按 `parent_session` 归并；
- 工作台主线切换 = 记录到 workbench.toml（会话本身不动）。

## 4. 修改请求（ChangeRequest）流程

定义：当某成员（或其 agent 回合）产生的文件变更落在**他人的作用域**
glob 内时，这些变更不直接生效，而是打包成 CR 发给作用域主人。

```
变更发生 ──▶ 按路径匹配 scope ──▶ 全部在自己作用域？
                                      │ 否
                                      ▼
                    打包 CR：{ files[], diff, base_snapshot_hash,
                               undo_ref, from, to_scope, reason }
                                      │ 经 broker 投递（W2 先走文件交换：
                                      ▼   .firment/cr/pending/<id>.json）
              主人工作台收到卡片：diff 预览 + 检查单
                      ├─ 批准 → 在主人分支上应用（快照未变则干净应用）
                      └─ 拒绝/评论 → 附言退回
```

**冲突策略 v1（刻意从简）**：CR 附带 base 快照哈希；应用时目标内容哈希
不一致 → 不自动合并，转为在双方之间开一条支线对话人工处理。三方合并列
远期（依赖真正的 rebase 编排，不值得早期投入）。

**agent 侧行为**：模型越界编辑时，工具层返回
`[ScopeGuard] 该路径属于 bob 的作用域，已生成修改请求 CR-xxxx`
而不是静默写入——模型据此向用户说明并继续其余工作。

## 5. 事件总线（与 sbc-agent 共享）

单一 mosquitto broker（Cubie A7A），主题命名空间统一约定：

| 主题 | 发布者 | 内容 |
|---|---|---|
| `firment/device/<node>/telemetry` | MCU/采集器 | 统一帧（sbc-agent §3.2）|
| `firment/device/<node>/alert` | 预过滤器/守卫 | 命中事件（升级给大模型的候选）|
| `firment/guard/status` | 守卫服务 | 待机/在线、最近动作、统计 |
| `firment/collab/presence` | 各端 GUI/CLI | 成员在线与所在支线 |
| `firment/collab/cr/<id>` | 任意端 | CR 提交/批准/拒绝事件 |

- GUI 的 `CollabBackend` trait 增补 `Device` 事件变体，实现
  `MqttCollabBackend`（W3）；Web 工作台经 MQTT-over-WS(:9001) 订阅同样的流；
- 单人模式下 broker 可整体缺席：工作台读本地文件状态即可，MQTT 只在
  多端/守卫场景需要。

## 6. 分期与验收

| 期 | 范围 | 验收标准 |
|---|---|---|
| **W1 单人核心** | WorkbenchView 替换 CollabView 槽位；主线+支线树（含 MetaLine 扩展）；快捷指令（内置+自定义）；仓库状态卡；ELF 预算卡；验证徽章；变更时间线；workbench.toml 读写 | 日常开发全程可在工作台完成，不再需要翻独立菜单 |
| **W2 作用域+审查** | scopes/CR 流（文件交换版）；引脚表+periph_init 联动；知识库入口；决策记录；CR 检查单 | 两人在同一仓库并行不踩车；出界变更必走 CR |
| **W3 无线+守卫** | MqttCollabBackend（presence/Device 流）；烧录历史；硬件清单；通知中心；守卫组件对接 sbc-agent 采集器（待机时长配置、升级事件流、统计） | 手机/第二电脑看到实时设备面板；守卫无人值守升级可用 |
| **W4 手机镜像** | Web 端只读工作台页（MQTT-over-WS + 现有 API Key 鉴权） | 手机浏览器查看项目状态/设备面板/通知 |

### 已决事项（2026-08-22 定稿）

1. **支线继承范围**：仅自动注入与支线 title 语义相关的决策记录；todo
   不继承（支线自带独立清单）。需要更多上下文时由用户显式 `/pin`。
2. **CR 粒度**：以 agent 回合为打包单位，文件列表与 undo_ref 随附——
   回合是变更的天然原子单位，按文件拆分会碎片化审批。
3. **引脚冲突判定 v1**：仅警告 + 工作台高亮提示，**不阻断**
   periph_init 生成。冷启动阶段归属数据不全，误报阻断会适得其反；
   运行一版后依据误报率再决定是否升级为阻断。
4. **成员身份来源**：v1 纯手工维护 workbench.toml 的 `[scope.*]`，
   无服务端依赖（符合"仓库即状态"原则）；接入统一用户体系列为远期。

## 7. 明确不做（本期边界）

自研版本存储、实时控制闭环、公网暴露的服务端、非 git 仓库的工作台支持。
