# SBC 端侧模型部署手册（从零到验收）

> 适用平台：Debian 系 SBC + systemd（实测：Radxa Cubie A7A）。
> 目标：一台全新的 SBC + 一台 PC + 一个 MQTT 节点固件 → `firm --doctor --sbc` 全绿。
> 全文所有命令都在真实部署中验证过；踩坑记录见 §7。

## 1. 架构总览

```
┌─────────┐  WiFi/MQTT   ┌──────────────── SBC ────────────────┐   LAN    ┌────── PC ──────┐
│ 节点固件 │─────────────▶│ mosquitto :1883 ◀── sbc-guard 守卫  │◀────────▶│ firm CLI / GUI │
│ ESP32-S3 │  telemetry/  │        ▲                │            │  config  │  provider:     │
│ (node)   │  cmd/state   │        │                ▼            │          │  sbc-ollama    │
└─────────┘              │        └──────── ollama :11434 ──────│─────────▶│  task 派发子代理│
                         │                   qwen2.5:0.5b 分类器 │  /v1 API └────────────────┘
                         └──────────────────────────────────────┘
```

| 组件 | 角色 | 形态 |
|---|---|---|
| mosquitto | MQTT broker，全网数据面汇聚点 | apt 包，systemd |
| ollama | 端侧模型服务（OpenAI 兼容 `/v1`） | 官方安装脚本，systemd |
| sbc-guard | 遥测采集、规则预过滤、小模型分类、两阶段告警、心跳 | `guardd.py` + systemd（`sbc-guard.service`） |

数据面主题约定（全部在 `firment/` 命名空间下）：

```
firment/device/<node>/telemetry   节点→SBC 遥测行
firment/device/<node>/alert       SBC→PC 告警（QoS1）
firment/device/<node>/state       节点状态帧（含命令 ack）
firment/device/<node>/cmd         PC→节点 命令（JSON 信封 v1）
firment/guard/status              守卫心跳（retained，10 分钟节拍）
```

## 2. 前置条件

- SBC：Debian 系，Python ≥3.9，能上外网（装包/拉模型），2GB+ 内存
- PC：已安装 firm CLI（`firm install`），与 SBC 同一局域网
- 网络：**强烈建议先在路由器把 SBC 的 MAC 做 DHCP 静态绑定**。IP 漂移会同时打断 broker、ollama、固件三处配置——我们已中招两次（.6→.8）

## 3. SBC 侧部署

### 3.1 mosquitto

```bash
sudo apt install -y mosquitto mosquitto-clients
sudo tee /etc/mosquitto/conf.d/firment.conf <<'EOF'
# LAN 明文监听：节点与 PC 直连内网，靠网络边界隔离
listener 1883 0.0.0.0
allow_anonymous true
EOF
sudo systemctl restart mosquitto && sudo systemctl enable mosquitto
```

> 内网明文是有意取舍：节点固件不带 TLS 栈时最简可行。若 broker 需要暴露到
> VLAN 之外，改用 8883 + 用户名密码并同步修改 PC `[mqtt]` 与固件常量。

### 3.2 ollama + 模型

```bash
curl -fsSL https://ollama.com/install.sh | sh
systemctl status ollama                    # 安装脚本自带 systemd，确认 active
ollama pull qwen2.5:0.5b                   # ~400MB，守卫分类器（生产默认）
ollama pull qwen3.5:0.8b                   # ~600MB，task 子代理用（thinking 模型）
curl -s http://127.0.0.1:11434/v1/models   # 应列出刚拉的两个模型
```

两个模型的分工（实测结论）：

- **qwen2.5:0.5b**：非 thinking、JSON 格式可靠、CPU 快 → 守卫分类器。
  ⚠ 但语义严重度判断不可信（基准 sev 0/10）——生产中规则层才是权威，
  LLM 只做 summary/category 细化，见 §7-P6。
- **qwen3.5:0.8b**：质量更高但每轮输出长 `<think>` 流，吃 max_tokens 预算
  → 适合异步 task 子代理，不适合实时分类。

### 3.3 guardd 守卫

```bash
sudo apt install -y python3-pip
# paho 必须 >=2.0（v1 无 CallbackAPIVersion）；tomli 仅 Python<3.11 需要
pip3 install --user "paho-mqtt>=2.0" requests "tomli; python_version < '3.11'"
mkdir -p ~/sbc-guard
```

从 PC 推送三个文件（**不要**用 ssh 内联长命令——Windows 引号转义是重灾区，一律 scp 文件再执行）：

```powershell
scp sbc-guard/guardd.py,sbc-guard/rules.toml,sbc-guard/config.toml radxa@<SBC-IP>:sbc-guard/
scp sbc-guard/sbc-guard.service radxa@<SBC-IP>:sbc-guard/
ssh radxa@<SBC-IP> "sudo cp ~/sbc-guard/sbc-guard.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now sbc-guard"
journalctl -u sbc-guard -f          # 期望看到 "[guard] up ... rules=N" 与 "[mqtt] connected rc=Success"
```

`config.toml` 关键字段（broker 走本机回环，无需随 IP 漂移而改）：

```toml
broker_host = "127.0.0.1"     # guardd 与 broker 同机
[ollama]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "qwen2.5:0.5b"        # 分类器
[guard]
standby_minutes = 10          # 心跳节拍；doctor 用 2× 该值判心跳新鲜度
escalate_sev = "warn"         # 达到该级别的告警升级到桌面
```

## 4. PC 侧配置

`firm config` 打开全局配置（`%APPDATA%\firment\config.toml`），加两段：

```toml
[mqtt]
broker = "<SBC-IP>:1883"

[providers.sbc-ollama]
type = "openai"
base_url = "http://<SBC-IP>:11434/v1"
model = "qwen3.5:0.8b"        # task 子代理默认用的模型
```

- key：本地 ollama 不需要鉴权；若端点有鉴权则 `firm --set-key sbc-ollama sk-xxx`
- agent 里说"用小模型跑个分类"，AI 会先用 `models` 工具发现端点上的模型
  （qwen2.5:0.5b / qwen3.5:0.8b 都可见），再通过 `task {provider, model}` 派发

## 5. 节点固件

模板：`docs/examples/esp32c3-mqtt-node/esp32c3-mqtt-node.ino`（C3/S3 SuperMini 通吃）。
编译期常量（改完烧录）：

| 常量 | 含义 | 示例 |
|---|---|---|
| `WIFI_SSID` / `WIFI_PASS` | 2.4G WiFi | `"lab"` / `"…"` |
| `MQTT_HOST` | **SBC 的 IP**（不是回环！IP 漂移要跟着改） | `"192.168.1.8"` |
| `NODE_NAME` | 节点名 = 板名 = pinmap board 名 = workbench.toml `[devices]` 键 | `"s3-node-1"` |

协议 v1 要点：节点上线发布能力帧（caps），PC 下发 JSON 信封
`{"v":1,"cmd":"rgb.set","params":{...},"id":...}`，节点回 state 帧带同 id ack。

## 6. 验收标准

```text
$ firm --doctor --sbc
  ✓ [mqtt] broker = 192.168.1.8:1883
  ✓ tcp 192.168.1.8:1883 reachable
  ✓ mqtt CONNACK
  ✓ guard heartbeat fresh (age 45s, beat 10min)
  ✓ sbc-ollama: model 'qwen3.5:0.8b' ready (2 served)
  ✓ devices bound: s3-node-1
```

六项全 ✓ 即验收通过。任何一项失败都带修复提示；配合 §7 对照表定位。

## 7. 故障排查（真实踩坑记录）

| # | 症状 | 根因 → 处置 |
|---|---|---|
| P1 | tcp refused | mosquitto 未起：`sudo systemctl status mosquitto`；确认 conf.d 监听 1883 |
| P2 | 心跳 STALE | guardd 崩溃循环：`journalctl -u sbc-guard -n 20`。历史上两次根因：paho v1 缺 `CallbackAPIVersion`；`clean_session=False` 未给 client_id（必须显式 id） |
| P3 | 服务名找不到 | 单元名是 **`sbc-guard.service`**，不是 firment-guard |
| P4 | ssh 改配置后服务读不懂 | Windows 编辑器写入 UTF-16 BOM。一律 UTF-8 无 BOM：PowerShell 用 `[IO.File]::WriteAllText($path,$t,(New-Object Text.UTF8Encoding($false)))` |
| P5 | ollama 探测超时 | 冷启动 ~70s（模型载入内存）；doctor 报 timeout 先原样重试一次 |
| P6 | revised 告警严重度离谱 | qwen2.5:0.5b 语义 sev 不可信（基准 0/10）。规则层的 sev 才是权威；LLM 输出仅作 summary/category 参考，微调语料积累于 `~/sbc-guard-data/pairs/` |
| P7 | qwen3.5 回复内容为空 | thinking 流吃光 max_tokens → 给足预算（guardd 已设 800 并剥离 `<think>` 块） |
| P8 | 全链路突然失联 | 十有八九是 DHCP 漂移：路由器做静态绑定（§2），然后同步改 PC `[mqtt]`、provider base_url、固件 `MQTT_HOST` 三处 |
| P9 | sudo 在 ssh 里卡住 | 非交互 sudo 要密码：配 SSH 公钥免密 + `radxa ALL=(ALL) NOPASSWD` 白名单，或交互终端执行 |
| P10 | pip 装完 import 还是失败 | Python≥3.11 不要装 tomli；paho 必须真升级到 v2（`pip3 show paho-mqtt` 看 Version） |

## 8. 附录：命名与服务标准

| 对象 | 标准 | 示例 |
|---|---|---|
| 节点名 | `<板名>-node-<序号>`，四处一致（固件 NODE_NAME / pinmap board / [devices] 键 / 告警 node 字段） | `s3-node-1` |
| SBC provider | `sbc-` 前缀 + 后端名；models 工具按 base_url host == broker host 自动识别端侧端点 | `sbc-ollama` |
| systemd 单元 | `sbc-guard.service`，`Restart=always RestartSec=5`，`MemoryMax=256M` 防失控 | |
| 微调语料 | `data_dir/pairs/<YYYYMMDD>.jsonl`，四元组 `(line, rule_sev, llm_*, published_sev)` | |

---
*维护说明：本文档与代码同步维护——改 guardd 部署形态、主题约定或协议版本时，请同步更新对应章节与 `firm --doctor --sbc` 的提示文案。*
