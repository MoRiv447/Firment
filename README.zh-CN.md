# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.3.0--beta.7-orange)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![Benchmark](https://img.shields.io/badge/benchmark-4.95-%231-green)]()

[English](README.md) | **简体中文**

> ⚠️ **状态：半成品，活跃开发中。**
> Firment 目前还只是半成品。第一层（通用编码 Agent）已经能跑、也做了测试，但 TUI、配置格式和工具接口都还在快速演进，随时可能调整，暂时不建议用于生产或关键任务。欢迎拿来试用，遇到问题请告诉我们。

**Firmware + Agent = Firment**——一个面向固件与嵌入式开发的通用编码 Agent。名字取自 *firmament*（苍穹），故意少一个 a，把 firmware + agent 融成一个词。第一层（通用编码 Agent 层）当前可用，但整体仍是半成品；后续构建、烧录、调试、UART 等层通过统一的 `Tool` trait 接入同一内核。

---

**分层定位。** 第一层（当前版本）是通用编码 Agent，与其他终端编码 Agent 同类，但在设计上为固件/嵌入式工作流预留接口。嵌入式专属能力（构建、烧录、调试、UART）属于**第二层**，正在开发中，详见 Roadmap。
### ✨ 特性**第一层 — 通用能力**

- **多模型接入**：Anthropic 兼容（`/v1/messages`）与 OpenAI 兼容（`/chat/completions`，覆盖 DeepSeek / GLM / Qwen / Ollama）流式工具调用；DeepSeek V4 自动走官方 `thinking` + `reasoning_effort`
- **思考深度分级**：`off / low / medium / high / xhigh / max`
- **内置工具**：`read_file`、`write_file`、`edit_file`（锚点/行范围编辑）、`list_dir`、`glob`、`grep`、`shell`
- **只读 Plan 模式**：`--plan` / `/plan` 只暴露读工具，plan 提示词要求“决策完整、执行者零决策”
- **并行工具调用**：独立工具并发执行；同文件读写与 shell/verify/grep 等宽泛工具自动排序
- **工程化系统提示词**：分节内建（沟通 / 工程原则 / 工具策略 / 验证 / 安全）+ `AGENTS.md` / `FIRMENT.md` 项目指令注入
- **会话管理**：JSONL 持久化、`--continue`、`--list`、TUI `/sessions` 上下键选择器
- **输出复制**：鼠标左键选择 + 右键复制（无选区时粘贴），`Ctrl+Shift+C` / `/copy` 复制最后回复
- **全局安装**：`firm install` 写用户 PATH + PowerShell 补全；`firm update` 自更新

**上下文管理**

- **模型摘要压缩**：超预算时由主模型把旧轮次压成摘要（本地摘要兜底），最近 3 轮逐字保留，绝不拆散工具调用配对
- **上下文压缩**：长会话自动把早期消息压缩成摘要（`context_budget_chars`）
- **缓存稳定前缀**：系统提示词保持字节不变以命中 Provider 前缀缓存；动态状态（改动台账）以增量合并进用户消息
- **重复读取去重**：未变化的文件重复读取时返回桩引用而非重复内容；压缩后自动回填最近读取的文件
- **Pin 固定**：`/pin <路径>` 标记文件在压缩时保留全文（逐字回填）；`/unpin <路径>` 取消
- **工具输出外溢**：超长工具输出自动落盘到会话外溢目录，对话里只留短摘录 + `read_file` 路径指针
- **改动台账**：每回合已提交的改动（路径/行数/hunk）写入会话台账，恢复时注入上下文；`/ledger` 查看
- **符号索引**：定义/引用查找自动优先 universal-ctags（JSON 输出），未安装时回退内置正则扫描；`[tools] symbols_backend = auto | ctags | regex`（Plan 模式也可用）

**安全与可靠性**

- **事务编辑 + 撤销**：一个回合内的所有写/编辑统一备份，任一修改失败整批回滚；`/undo` 恢复上一次已提交的改动（按会话持久化）
- **CAS + SHA-256 哈希锚定**：写/编辑前逐字节重新校验；`read_file` 返回 `[file-sha256: ...]`，`edit_file` / `write_file` 支持 `expected_sha256` 前置校验，哈希不符以 `[ConcurrentChange]` 拒绝
- **diff-first 批准**：写/编辑的权限弹窗直接展示 unified diff，批准前先看改动
- **verify 硬门**：可选 `verify` 工具执行配置的构建/检查命令；发生文件改动后由程序强制运行 verify，未通过就不接受完成
- **路径沙箱**：文件工具被限制在工作区内（canonicalize 校验，外溢目录等额外根目录显式放行），越界路径以 `[Permission]` 拒绝
- **危险命令安全闸**：`-y` 一次性模式下默认拦截 `del/rm/Remove-Item/mv/move/git clean/git reset --hard` 及脚本删除 API，防止包装绕过；TUI 中标注 ⚠ 并弹权限确认
- **参数 schema 校验**：工具参数在执行前按 JSON Schema 校验，非法参数以 `[InvalidInput]` 拒绝
- **失败分类**：工具错误带 `[NotFound]`、`[CompileError]`、`[Timeout]`、`[Permission]`、`[ConcurrentChange]` 等标签

**第二层 — 嵌入式专属（编译/烧录已可用，其余开发中）**

- 编译 / 烧录 / 运行 / 监控：`firm build`、`firm flash`、`firm run`（probe-rs RTT 日志）与 `firm monitor`（串口 + ELF 符号解码）现已可用
- 调试（probe-rs）、UART / 串口日志分析（含 ELF 符号解码栈回溯）
- MCU 自动识别（`.ioc` / CubeMX 芯片库）
- 寄存器 / 外设感知（芯片寄存器映射、`.ioc`、设备树）
### 🚀 快速开始

环境要求：Rust 1.85+，推荐 Windows Terminal 或任意现代终端。

一行安装（无需本地 Rust 工具链）：

```powershell
# Windows
irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1 | iex
```

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
```

国内加速：先设置镜像根地址再执行安装脚本（目录结构 `{mirror}/{tag}/{asset}`，例如阿里云 OSS）。

> **安全说明**：安装脚本从 GitHub Releases 下载二进制，并在运行前用该 release 的 `SHA256SUMS` 做 SHA-256 校验；脚本本身很小、走 HTTPS、可在本仓库审阅。想先预览不执行，在运行一行命令前设置 `FIRMENT_DRY_RUN=1`；需要固定版本用 `FIRMENT_VERSION`。
从源码构建：

```powershell
cargo build --release
.\target\release\firm install      # 安装到 PATH，之后新开终端直接输入 firm
firm --doctor                       # 检查配置、Provider 连通性与安装状态
firm                                # 进入交互式 TUI
firm -p "把 src/main.rs 里的 greet 函数改成打印 Hello"
```

升级新版本（从构建目录运行，避免覆盖正在运行的安装文件）：

```powershell
cargo build --release
.\target\release\firm update
```

### ⚙️ 配置 API

首次运行自动生成 `%APPDATA%\firment\config.toml`（Unix 为 `~/.config/firment/config.toml`，可用 `FIRMENT_CONFIG_DIR` 或 `--config` 指定）。默认 Provider 指向 DeepSeek V4（`deepseek-v4-flash`）；没配 key 也能进 TUI，`/apikey sk-xxx` 即可。

```toml
[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"   # 或 deepseek-v4-pro

# thinking = "medium"      # off / low / medium / high / xhigh / max
# context_budget_chars = 60000       # 会话上下文字符预算，超出自动压缩早期对话
# compaction_strategy = "summarize"  # 默认 summarize；可选 drop（超预算直接丢弃旧轮）/ off（不自动压缩）

[tools]
# verify_command = "cargo check"   # 改动后先跑通再宣布完成（如 cmake --build build）
# symbols_backend = "auto"         # auto / ctags / regex（符号索引后端）
# build_command = "cmake --build build"   # build 工具（Keil: uv4 -j0 -b project.uvprojx）
# default_chip = "stm32f407vetx"          # firm flash 默认芯片（probe-rs 芯片名）
# monitor_port = "COM3"                   # firm monitor 默认串口
# monitor_baud = 115200                   # firm monitor 默认波特率
```

多 Provider 追加配置后用 `--provider <名字>` 或 TUI 内 `/provider <名字>` 切换；`/models`、`Ctrl+P` 可直接拉取并选择模型，不用手改文件。

### 📚 硬件知识库（可选）

在固件项目里放 `docs/vendor-index.toml`（外加 `docs/cheatsheets/` 原创速查表），Firment 会自动发现，并在提示词里要求 agent 优先查询；涉及芯片/外设/寄存器/HAL 的问题会先查知识库再作答。模板见 [docs/vendor-index.toml](docs/vendor-index.toml)，说明见 [docs/vendor-index.md](docs/vendor-index.md)。

### 📁 项目级配置（让 AI 自己干活）

在项目根目录放 `.firment.toml`，把构建/烧录/串口配置写进 `[tools]`（可提交进版本库）：

```toml
[tools]
build_command = "cmake --build build"   # 或 uv4 -j0 -b project.uvprojx
default_chip = "stm32f407vetx"
monitor_port = "COM3"
```

项目配置会覆盖全局 `config.toml` 的对应项。进 TUI 后直接说“构建并烧录”，agent 会自己读取/修改这份文件并调用 `build` / `flash` / `run`；`build` 默认免确认，`flash` 始终弹确认。

### 🖥️ 命令行

| 命令 | 说明 |
|---|---|
| `firm` | 交互式 TUI |
| `firm -p "任务"` | 单次执行 |
| `firm --plan -p "调研并给出实现计划"` | 只读 Plan 模式 |
| `firm -y -p "任务"` | 自动批准写/编辑/shell |
| `firm -y --allow-dangerous -p "任务"` | 放行危险 shell 命令（默认拦截） |
| `firm --continue [<id>]` | 恢复最近/指定会话 |
| `firm --thinking xhigh -p "任务"` | 指定思考深度 |
| `firm --list` / `firm --doctor` | 会话列表 / 配置+安装检查 |
| `firm install` / `firm update [<exe>]` | 全局安装 / 自更新 |
| `firm build` | 执行配置的构建命令（`[tools] build_command`，如 CMake/Make/Keil/IAR 命令行） |
| `firm flash [--chip <芯片>] <elf>` | 用 probe-rs 烧录固件（ST-Link / J-Link / CMSIS-DAP / DFU） |
| `firm run [--chip <芯片>] [--timeout <秒>] <elf>` | 烧录并复位运行目标，流式输出 RTT 日志 |
| `firm monitor [--port <COMx>] [--baud <波特率>] [--elf <elf>]` | 串口监控；带 `--elf` 时对日志中的栈地址做符号解码 |
| `firm --set-key default=sk-xxx` | 写入 API key |

### 🎮 TUI 交互

斜杠命令：`/plan [on|off]`、`/agent`、`/models`、`/model <id>`、`/sessions`（↑/↓ 选择）、`/session <id>`、`/undo`、`/ledger`、`/pin <路径>`、`/unpin <路径>`、`/provider <名字>`、`/add-provider`、`/apikey`、`/thinking`、`/copy`、`/config`、`/clear`、`/help`、`/quit`。

键位：`↑/↓` 空输入时浏览历史、非空时滚动；`PgUp/PgDn`/滚轮始终滚动；`Ctrl+P` 模型选择器；鼠标左键选择 + 右键复制（无选区时粘贴）；`Ctrl+Shift+C` 复制最后回复；`←/→`、`Home/End`、`Ctrl+A/E` 移动光标；权限弹窗 `y`/`n`/`a`。

### 🔒 安全模型

- **免责声明**：危险命令安全闸是基于命令名扫描的 best-effort 启发式拦截，不是操作系统级沙箱。文件工具由路径沙箱约束；`shell` 仅靠权限确认兜底。需要强隔离请在容器/VM 中运行。
- 写文件 / 编辑 / shell 默认需要权限确认（TUI 弹窗，`y`/`n`/`a`）
- `-y` 自动批准模式仍受**危险命令安全闸**约束：`del/erase/rm/rmdir/rd/Remove-Item/mv/move/ren/git clean/git reset --hard/强推/format/taskkill` 以及脚本删除 API 全部拦截，需要显式 `--allow-dangerous`
- Plan 模式只暴露只读工具，权限层再硬拒写/编辑/shell
- 系统提示词内置“忠实汇报”约束：运行过的命令必须照实描述，禁止声称操作被“完全拦截”而实际已改变工作区

### 🏆 横向评测（2026-08-07）

在五家通用编码 Agent 横向评测（19 用例 × 5 agent，同一 `deepseek-v4-flash` 模型、one-shot 模式）中，Firment 以 **4.95 分位列第一**：

| Agent | 加权总分 |
|---|---|
| **Firment** | **4.95** |
| Codex | 4.88 |
| Claude Code | 4.60 |
| opencode | 4.55 |
| oh-my-pi | 4.30 |

评测口径与明细见 [BENCHMARK.md](BENCHMARK.md)。S1（危险删库）经三轮安全闸加固后成为五家唯一做到“先警告、求确认”的 agent。

### 📦 项目结构

```text
crates/
  firment-core/   Provider 抽象、Agent 循环、会话、配置、权限、Tool trait、系统提示词
  firment-tools/  内置文件/搜索/shell 工具（含危险命令安全闸）
  firment-tui/    ratatui 终端界面（选择复制、会话/模型选择器）
  firment-cli/    clap 入口（bin: firm）+ 安装/更新/补全
```

### 🧪 开发

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### 🗺️ Roadmap

- 第二层：构建系统集成（CMake/Make/Keil/IAR）、烧录与调试（OpenOCD/ST-Link）、UART/日志
- 语法感知：tree-sitter 结构化编辑与补全
- 插件 / MCP：统一工具注册表上开放第三方扩展
- Web / 云端：Rust 后端容器化 + 可选 Web 前端

### 🤝 贡献

欢迎 Issue、PR 和评测反馈。请先运行质量门三项并附上对应测试。

### 📄 许可证

[MIT](LICENSE) © 2026 MoRiv447

### 🙏 致谢

架构与体验参考了 [opencode](https://github.com/anomalyco/opencode)、[pi](https://github.com/earendil-works/pi) 与 [oh-my-pi](https://github.com/can1357/oh-my-pi)，感谢这些优秀开源作品。
