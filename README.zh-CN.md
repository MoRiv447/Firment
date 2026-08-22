# Firment — Firmware + Agent

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.5.13-blue)](Cargo.toml)
[![Rust](https://img.shields.io/badge/rust-1.85+-deeppink)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)]()
[![CI](https://img.shields.io/badge/CI-Rust%20%2B%20Web%20%2B%20GUI-green)](.github/workflows/ci.yml)

[English](README.md) | **简体中文**

<p align="center">
  <img src="docs/screenshots/cli.png" alt="Firment TUI：ask_user 弹窗 + 工具调用栈 + 底部状态栏（git 分支 / 思考级别 / Esc×2 中断提示）" width="900">
</p>

> ⚠️ **状态：v0.5.13。** 第一层（通用编码 Agent）已可日常使用、
> CI 全绿；第二层（嵌入式工具链闭环）已落地——构建/烧录/运行、串口监控、
> ELF 分析、probe-rs 片内调试均可用——调试器纵深（栈回溯、SWO trace）
> 在路线图中。接口与 TUI 仍在演进。

**Firmware + Agent = Firment**——一个面向固件与嵌入式开发的通用编码
Agent。名字取自 *firmament*（苍穹，每个嵌入式工程师头上的那片天），故意
少一个 a，把 firmware + agent 融成一个词。内核是一个 Rust 编码 Agent，
带着嵌入式优先的工具链闭环：写代码、编译、烧录、运行、串口监控、ELF
分析——全部在同一次对话里完成。

**一个仓库，三端入口（monorepo）：**

| 端 | 路径 | 技术栈 |
|---|---|---|
| **CLI / TUI** | [`crates/`](crates/) | Rust (ratatui)——核心 Agent |
| **GUI 客户端** | [`gui/`](gui/) | Tauri + React/Vite (TypeScript) |
| **Web** | [`web/`](web/) | Next.js + Tailwind（Vercel 部署）——在线体验：[firment-web.vercel.app](https://firment-web.vercel.app) |

CLI 是事实标准；GUI 客户端共用同一个 Rust Agent 内核（统一的 `Tool`
trait 和会话格式），Web 端是 TypeScript 重新实现，通过提交的工具规格
快照（`web/src/lib/tools/specs.json`）保持同步。

### ✨ 第一层特性（通用编码 Agent）

- **多提供商**：Anthropic 兼容（`/v1/messages`）与 OpenAI 兼容
  （`/chat/completions`，覆盖 DeepSeek / GLM / Qwen / Ollama）流式工具调用；
  DeepSeek V4 自动使用官方 `thinking` + `reasoning_effort`
- **思考级别**：`off / low / medium / high / xhigh / max`
- **内置工具**：`read_file`（带行号分页）、`write_file`、`edit_file`
  （锚点/行区间/hashline 编辑，回显统一 diff）、`list_dir`、`glob`、
  `grep`、`shell`、`web_search`（DuckDuckGo / Tavily / Brave）、`web_fetch`、
  `task`（只读研究子代理）、`todo`、`ask_user`、`hil`、`periph_init`、
  `elf_analyze`、`monitor`、`debug`
- **只读计划模式**：`--plan` / `/plan` 只暴露只读工具，要求给出可执行的完整计划
- **并行工具调用**：独立调用并发执行；同文件读写与粗粒度工具自动串行
- **工程级系统提示词**：沟通、工程原则、工具策略、验证、安全等分节，
  支持 `AGENTS.md` / `FIRMENT.md` 项目指令
- **会话管理**：JSONL 持久化、`--continue`、`--list`、交互式 `/sessions`
  选择器、变更台账 + `/undo`
- **复制支持**：左键拖选、右键复制、`Ctrl+Shift+C` 复制最后一条回复
- **全局安装**：`firm install` 加入 PATH + 补全；`firm update` 自更新

### 🛠️ 第二层——嵌入式工具链闭环（逐步落地）

- **`periph_init`** —— MCU 外设初始化骨架 + 知识库 cheatsheet。STM32
  （F1/F4/G0/**G4**/**H7**）完整 UART/GPIO/I2C/SPI/TIM/ADC HAL 骨架，
  ESP32/ESP32-S3 指引，CubeMX/PlatformIO HAL 重复警告。内置知识库覆盖
  真实工程坑：**G4 的 DMAMUX**（没有固定 DMA 通道——F1→G4 迁移经典坑）
  与 **H7 的 D-Cache 一致性**（DMA TX 前 clean、RX 后 invalidate）
- **`elf_analyze`** —— flash/RAM 占用、函数体积、`-fstack-usage` `.su`
  文件的真实栈深度；每次编辑回合后自动重新分析。增长超过配置阈值时，
  完成会被拦截，直到你批准（headless + `strict` 模式则直到模型修复）；
  低于阈值的变化默认静默吞掉
- **`monitor`** —— 串口监控，逐行时间戳 + 波特率自动检测；取消回合立即
  释放串口
- **`hil`** —— 硬件在环套件：一条命令串起 `build → flash → monitor`
  （带 `expect_contains`/`expect_regex` 断言）`→ elf_analyze`，支持
  `.firment/hil.toml` 套件或内联 steps、`dry_run` 模拟、JSONL 回放
  （`hil replay`）、串口/波特率自动检测与总超时——替代手动串联
  build/flash/monitor 的固件验证方式
- **`build` / `flash` / `run`** —— CMake/Make/Keil 构建命令、probe-rs
  烧录（芯片来自 `[tools] default_chip`），全部接入 Agent 循环
- **`debug`** —— 通过探针做完整的片内调试（基于 probe-rs，不依赖
  OpenOCD/GDB），Agent 可以自主调试自己写的固件：
  - `analyze` —— 一键故障诊断：暂停目标，读取 PC/LR/SP 与 Cortex-M 故障
    寄存器（CFSR/HFSR/MMFAR/BFAR），对照固件 ELF 解码 PC/LR（`func+0x12`），
    并逐项解释置位的故障标志（IACCVIOL / IBUSERR / UNDEFINSTR / FORCED /
    VECTTBL / STKOF ...）
  - `halt` / `regs` —— 暂停目标并读取完整寄存器表；两次调用之间目标保持
    暂停，直到烧录、复位或 `debug continue`
  - `mem` / `write` —— 内存读写，地址支持 `0x...` 或 `symbol:name`
    （从 ELF 符号表解析）；`write` 需要批准
  - `break` / `step` / `continue` —— 设置断点并在命中时上报寄存器、
    单步、恢复运行

### 🪟 三端入口，同一个内核

| 端 | 截图 |
|---|---|
| **GUI** —— Tauri + React；工具卡片实时流式（成功/失败红绿），状态徽章显示 `idle Ns` / `tool Nm Ns`；Agent 内核和 CLI 共用同一份 Rust 字节码 | <img src="docs/screenshots/ide.png" alt="Firment GUI：工具卡片（list_dir ✓、read_file ✕、todo ✓）与 running periph_init… tool 16s 状态条" width="900"> |
| **Web** —— Next.js，部署在 Vercel（`firment-web.vercel.app`）；同一套 Agent 内核 + `Tool` trait + tool-spec 快照，浏览器即可用 | <img src="docs/screenshots/web.png" alt="Firment Web：问「Search for GPIO configuration patterns」—— Agent 调起 grep/glob/list_dir/read_file 后诚实告知当前是 Next.js 工程没有固件代码" width="900"> |

### 🚀 快速开始

```bash
cargo build --release
./target/release/firm            # 开启新会话
./target/release/firm --continue # 恢复上次会话
```

Windows 一行安装（加入 PATH + PowerShell 补全）：

```powershell
Set-ExecutionPolicy -Scope Process Bypass; iex (irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1)
```

macOS / Linux：

```bash
curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
```

### ⚙️ 配置

`firm config` 打开配置文件（首次运行自动生成；Windows 在
`%APPDATA%\firment\config.toml`，其他系统在 `~/.config/firment/config.toml`）：

```toml
[provider.default]
base_url = "https://api.deepseek.com/v1"   # OpenAI 兼容端点
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"

thinking = "medium"        # off / low / medium / high / xhigh / max
context_budget_chars = 60000
compaction_strategy = "summarize"   # summarize / drop / off

verify_command = "cargo check"        # 声明完成前运行
symbols_backend = "auto"              # auto / ctags / regex
build_command = "cmake --build build" # Keil: uv4 -j0 -b project.uvprojx
default_chip = "stm32f407vetx"        # probe-rs 烧录芯片
monitor_port = "COM3"                 # 串口监控端口
monitor_baud = 115200
web_search = "duckduckgo"       # duckduckgo（免 key）/ bing（免 key，国内可达）/ tavily / brave
elf = "build/fw.elf"                  # 自动建立 elf_analyze 基线

# ELF 门禁完整策略（表格形式；上面的字符串形式使用默认值）：
# [tools.elf]
# glob = "build/fw.elf"
# stack_threshold = 32        # 栈深增长（字节）超过即拦截完成
# flash_threshold_kib = 1     # flash 增长（KiB）超过即拦截完成
# report_benign = false       # 是否把低于阈值的变化交给模型审查（false=吞掉）
# strict = false              # headless/CI：不降级，修到通过才放行
```

项目级配置（仓库内 `.firment/config.toml`）会叠加合并；模型也会被引导
阅读 `AGENTS.md` / `FIRMENT.md` 保持自律。

### 📚 硬件知识库（可选）

`periph_init` 使用随附的种子知识库（物化到配置目录）：`vendor-index.toml`
（芯片家族 ↔ 参考手册 ↔ cheatsheet 链接）+ `cheatsheets/*.toml`（原创工程
经验，已对照参考手册核验）。项目仓库可在 `.firment/` 旁放自己的
`vendor-index.toml`，模型会合并两者。

### 🖥️ CLI

```
firm           开始新会话
firm --continue 恢复上次会话
firm --plan    只读计划模式
firm /sessions 交互式会话选择器
firm install   加入 PATH + 补全
firm update    自更新
firm config    打开配置文件
firm build     运行配置的构建命令
firm flash     通过 probe-rs 烧录固件 ELF
firm run       烧录并运行目标，流式输出 RTT 日志
firm monitor   串口监控，可选 ELF 符号解码
firm tools     打印工具注册表 specs（JSON，唯一事实源）
```

### 🎮 TUI

- 状态栏显示模式、提供商/模型、思考级别、**git 分支 + 工作树变更数**
  （每 4 秒后台刷新，非 git 仓库自动隐藏）
- 运行中按 `Esc` 两次中断（带 5 秒确认窗口）；空闲时按一次清空输入
- 斜杠命令：`/new`、`/plan [on|off]`、`/agent`、`/models`、`/model <id>`、
  `/sessions`、`/session <id>`、`/undo`、`/ledger`、`/pin`、`/unpin`、
  `/provider`、`/add-provider`、`/apikey`、`/thinking`、`/budget`、`/output`、
  `/copy`、`/context`、`/config`、`/clear`、`/help`、`/quit`——完整命令与
  快捷键清单见 `/help`
- `Ctrl+P` 打开模型选择器；`↑/↓` 历史/滚动；`PgUp/PgDn` + 滚轮滚动；
  左键拖选 + 右键复制；`Ctrl+Shift+C` 复制最后回复

### 🔒 安全模型

- **声明**：危险命令防护是尽力而为的启发式（命令名扫描），不是操作系统
  沙箱。文件工具受路径沙箱约束；`shell` 仅靠权限确认。需要强隔离请在
  容器/虚拟机里运行 Firment。
- 写/改/shell 默认需要权限确认（TUI 弹窗，`y`/`n`/`a`）；`-y` 依然受
  危险命令防护约束（`rm/rmdir/del/erase/Remove-Item/mv/ren/git clean/
  git reset --hard`、强推、`format`、`taskkill`、脚本删除 API、cmd 风格
  `%VAR%` 间接调用——除非传 `--allow-dangerous`）
- 计划模式只暴露只读工具；权限层硬拒写/改/shell
- 事务化编辑 + undo 台账；内容寻址编辑（SHA-256）保证同一次编辑只会
  生效一次；路径沙箱 + spill 配额
- 系统提示词要求如实汇报：准确描述实际执行的命令，绝不在工作区已变更时
  谎称"已被完全拦截"

### 🏆 基准测试（2026-08-07）

四 Agent 基准（19 用例 × 4 Agent，同一 `deepseek-v4-flash` 模型，一次
生成模式），Firment 以 **4.95 分第一**：

| Agent | 加权分 |
|---|---|
| **Firment** | **4.95** |
| Codex | 4.88 |
| opencode | 4.55 |
| oh-my-pi | 4.30 |

方法与细节：[BENCHMARK.md](BENCHMARK.md)。三轮加固后，S1（危险删除）
中只有 Firment 会先警告并请求确认。

### 📦 项目结构

```text
crates/
  firment-core/   Provider 抽象、Agent 循环、会话、配置、权限、Tool trait、系统提示词、KB 种子
  firment-tools/  文件/搜索/shell 工具、危险命令防护、periph_init/elf_analyze/monitor/debug（第二层）
  firment-tui/    ratatui 终端界面（git 状态栏、模型/会话选择器）
  firment-cli/    clap 入口（bin: firm）+ install/update/补全
gui/              Tauri GUI 客户端（React/Vite + src-tauri）
web/              Next.js 前端（Vercel）
docs/             vendor-index.toml + cheatsheets/*.toml（硬件知识库）
.github/workflows/  CI：Rust（fmt/clippy/test）+ web-check + gui-check
```

### 🧪 开发

```powershell
cargo test               # 单元 + 集成测试（4 个 crate）
cargo clippy --all-targets -- -D warnings
cargo fmt --check
# web / ide
cd web && npm ci && npx tsc --noEmit && npm run build
cd ide && npm ci && npx tsc --noEmit && npm run build
```

### 🗺️ 路线图

- **调试器纵深**：栈回溯 / 反汇编、变量与表达式求值、SWO/trace 流式
  接入 Agent 循环
- TUI 命令面板（模糊查找）与流式 token 动画
- tree-sitter 结构化编辑与补全
- 基于统一工具注册表的插件 / MCP
- Web 后端：容器化 Rust Agent 支撑 Web 前端

### 🤝 贡献

欢迎 Issue、PR 与基准反馈。请先跑通三个质量门禁并附上相关测试。

### 📄 许可证

[MIT](LICENSE) © 2026 MoRiv447

### 🙏 致谢

架构与交互设计受 [opencode](https://github.com/anomalyco/opencode)、
[pi](https://github.com/earendil-works/pi) 与
[oh-my-pi](https://github.com/can1357/oh-my-pi) 启发。
