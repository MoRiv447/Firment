# SBC Agent 架构设计 —— 小模型数据桥（v1 草案）

> 状态：设计定稿，待硬件到位后按 §5 验证清单实施。
> 配套文档：`docs/gui-workbench.md`（工作台与守卫的呈现层）。

## 0. 背景与目标

Firment 的大模型 Agent 运行在桌面端，但嵌入式项目的"现场"在别处：
SBC 主控、多块 MCU 目标板、传感器与执行器。本设计引入一台常驻 SBC
（Radxa Cubie A7A 6GB），其上运行一个小模型（Qwen3.5-0.8B，经 Ollama
暴露 OpenAI 兼容端点），承担 **数据通信/协议翻译/摘要** 角色——让
firm 的大模型能够感知并操作整个嵌入式项目。

三端分工：

| 端 | 角色 |
|---|---|
| CLI/TUI | 大模型大脑；读事件文件决策、派发翻译子任务 |
| GUI 工作台 | 守卫状态卡、设备遥测面板、升级事件流（见 gui-workbench.md §W3） |
| Web/手机 | 只读镜像（二期） |

## 1. 角色与安全红线

1. **小模型无决策权。** 它只做三类事：
   - 规范化：原始日志/遥测 → 结构化 JSON 帧；
   - 摘要：把一段日志压缩成事件摘要；
   - 分类：判断一条输出是否值得升级给大模型。
   所有"动作"（烧录、写文件、重启外设）只能由大模型发起。
2. **双向 Schema 校验。** 小模型的输出必须通过 JSON Schema 校验才被
   采纳；不合格 → 丢弃并重问（带错误提示）。校验器复用
   `firment-core/src/schema.rs` 的思路。
3. **动作白名单。** 即使大模型发起，具体动作也须命中白名单
   （flash/run/monitor/read），未列出的拒绝。
4. **实时控制属于 MCU 固件。** LLM 只活动在"秒级观察—分钟级响应"
   层；微秒级闭环永远在固件里。

## 2. 硬件清单（当前）

| 设备 | 角色 |
|---|---|
| Radxa Cubie A7A 6GB | 常驻主控：mosquitto broker、Ollama+0.8B、采集器 |
| ESP32-C3 SuperMini ×n | 无线节点主力（原生 BLE5 + WiFi，¥10 级） |
| ESP32-S3 SuperMini | 同上 / 大带宽节点 |
| ESP32-S31（在途） | 未来 BLE central 网关位（NimBLE 聚合低功耗节点后走 WiFi 上报）；到货先做 §5 实测 |
| HC-08 模块（可选购） | 仅用于不可刷固件的旧 MCU（透传 UART→BLE） |
| USB 蓝牙 dongle（备件） | Cubie A7A 板载蓝牙支持度待实测，失败即用 dongle 兜底 |

## 3. 数据面设计

### 3.1 拓扑

```
[BLE 低功耗节点 (HC-08/NUS)]──BLE GATT notify──┐
[MCU (UART/USB-CDC)]───────────────────────────┤
[WiFi 节点 (ESP32, MQTT)]──────────────────────┤      [Cubie A7A]
                                               └────▶ mosquitto broker
                                                        │  (MQTT :1883 + WS :9001)
                                              采集器 ◀──┘  （订阅全主题）
                                                │
                                    环形缓冲/日志文件（全量落地，不丢）
                                                │
                                    确定性预过滤器（正则/阈值/关键词）
                                                │  仅命中事件
                                                ▼
                                     events.jsonl（结构化事件）
                                          ↘ 0.8B 规范化（可选增强字段）
```

- BLE 只是"末梢接入"，主干一律 MQTT-over-LAN；手机/Web/GUI 全部连
  broker，不直连节点（iOS 无 Web Bluetooth、BLE 吞吐 2~6 KB/s 不堪大任）。
- S31 到货后可作为 BLE central 网关：低功耗节点不必够得着 SBC。

### 3.2 统一帧格式

所有来源归一为同一种紧凑 JSON 帧（采集器写入 `events.jsonl`，每行一帧）：

```json
{"node":"cubie","ts":1755850000,"kind":"log","sev":"warn","src":"/dev/ttyUSB0","payload":"E (123) wifi: reconnect"}
```

- `node`：产生数据的节点名（MCU 板名或串口别名）
- `ts`：**SBC 到达时盖章**（MCU 时钟不可信）
- `kind`：log / metric / event / guard
- `sev`：debug/info/warn/error（由预过滤器初判，0.8B 可修订）

### 3.3 预过滤器规则优先（关键原则）

原始流**永不直接进入任何模型**：

1. 全量字节落地环形缓冲/按天日志（可回溯，不依赖模型记忆）；
2. 确定性规则（正则、阈值、关键词表）决定"什么值得看"——微秒级、零成本、
   行为完全可预测；
3. 只有命中事件才交给 0.8B 做结构化/摘要，然后进入 `events.jsonl`。

带宽账：一块 MCU 的传感器流可达每秒数 KB，而 0.8B 在 A7A 上约 15 tok/s
——模型跟得上"事件"，跟不上"流"。规则层就是两者的隔离带。

### 3.4 时间戳与时钟

MCU 不带 RTC 或时钟漂移时，一律以 SBC 接收时刻为准；MCU 自报 uptime 的，
由采集器换算为绝对时间并标注 `[est]`。

## 4. 控制面：firm ↔ 小模型

### 4.1 下行（firm → 0.8B）

- SBC 上 `ollama serve` 后即暴露 OpenAI 兼容端点
  `http://<sbc>:11434/v1`；
- 桌面端 `firm add-provider sbc-ollama http://<sbc>:11434/v1` 一次配置；
- `task` 工具已支持 `provider` + `model` 双覆盖（见
  `feat(tools): task subagent provider override`）：大模型可把翻译/摘要/
  调研类子任务显式派发给 0.8B，主循环保持大模型。

### 4.2 上行（SBC → firm，v1）

- 事件即文件：`events.jsonl` 放项目 `.firment/work/events-YYYYMMDD.jsonl`；
- firm 通过 `shell`（ssh 别名）或 rsync 同步后 `read_file` 读取；
- 二期再考虑 HTTP API（需给 `web_fetch` 加私网白名单开关）与 MCP 工具桥。

### 4.3 安全

- broker 与 Ollama 端口仅绑定局域网接口；远程访问走 Tailscale/WireGuard，
  不做公网暴露；
- Ollama 开启 `OLLAMA_ORIGINS` 白名单；
- 小模型输出永远过 Schema 校验（§1.2），失败不计入事件流。

## 5. P0 硬件验证清单（Cubie A7A 到手后照此执行）

| # | 步骤 | 通过标准 |
|---|---|---|
| 1 | 刷官方 Debian/Ubuntu ARM64，`apt install mosquitto mosquitto-clients` | 服务自启 |
| 2 | mosquitto 配置追加 `listener 9001` + `protocol websockets`（装 libwebsockets 构建） | `ss -ltn` 见 1883/9001 |
| 3 | `curl -fsSL https://ollama.com/install.sh \| sh` && `ollama pull qwen3.5:0.8b` | `curl http://127.0.0.1:11434/v1/models` 列出模型 |
| 4 | ESP32-C3 SuperMini 烧 PubSubClient 示例，发布 `hello` 到 `firment/test` | 桌面 `mosquitto_sub -h <sbc> -t 'firment/#'` 收到 |
| 5 | 板载蓝牙检测：`bluetoothctl list`；无控制器或不稳 → 插 USB dongle 重试 | `hciconfig` UP |
| 6 | Rust `bluer` 扫描示例发现 HC-08/ESP32 BLE 广播 | 列出设备名 |
| 7 | （S31 到货）`probe-rs list` 识别 + 尝试 `firm flash --chip esp32s31`；失败转 espflash 路线并回填本文档 §2 注记 | 记录结论 |

## 6. 训练课程（Qwen3.5-0.8B 专用微调路径）

> 原则：**先不训**。窄域格式翻译靠提示词 + Schema 校验大概率够用；
> 微调是提示词失效后的手段，不是起点。

| 阶段 | 动作 | 退出条件 |
|---|---|---|
| 第 0 步 | 固定系统提示词模板（任务定义 + 3~5 个 few-shot 输入/输出对 + 输出 Schema） | 校验合格率 ≥ 90% 即长期不训 |
| 第 1 步 | **自动攒对**：每次校验通过的（输入，规范化输出）落盘 `pairs/`；人工修正的样本单独标记 | 持续积累 |
| 第 2 步 | 达到 **500~2000 对** 且合格率停滞 <90% → 启动训练 | — |
| 第 3 步 | QLoRA 微调：LLaMA-Factory（网页界面，最适合首训）或 Unsloth 免费 Colab T4；数据格式=标准 chat JSONL；0.8B 单卡半小时级 | 验证集指标提升 |
| 第 4 步 | **验收**：留出 10% 测试集，两项硬指标——Schema 合格率、字段精确匹配率——均须超过基线提示词版本 | 不过 → 回炉数据 |
| 第 5 步 | 合并 LoRA → 转 GGUF Q4_K_M → Ollama Modelfile 部署；firm 侧零改动 | — |

## 7. 二期路线（本期不做）

- 采集器常驻服务化（systemd unit）+ 多端口自动发现；
- `web_fetch` 私网白名单开关 → 直接拉 SBC 事件 API；
- MCP 工具桥：把 0.8B/采集器封装为 MCP server，firm 以 client 接入；
- GUI 工作台守卫组件（对接 `gui-workbench.md` §W3）；
- Tailscale 远程访问方案。
