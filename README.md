# firment — Firmware + Agent

第一层通用编码 Agent（Firment，Beta）的完整版实现：Rust 单体核心 + 交互式 TUI + 单次执行 CLI + 全局安装/自更新 + 只读 Plan 模式。

## 功能

- 模型接入：Anthropic 兼容（`/v1/messages`）与 OpenAI 兼容（`/chat/completions`，覆盖 DeepSeek/GLM/Qwen/Ollama）流式工具调用；DeepSeek V4 自动走官方 `thinking` + `reasoning_effort`（high/max）参数
- 思考深度：Claude Code 风格 `off / low / medium / high / xhigh / max`，Anthropic 走 extended thinking，OpenAI 兼容走 `reasoning_effort`（GPT 支持 xhigh，DeepSeek 映射到 max）
- 内置工具：`read_file`、`write_file`、`edit_file`（锚点/行范围编辑）、`list_dir`、`glob`、`grep`、`shell`
- 只读 Plan 模式：`--plan` / `/plan` 下只暴露读工具（read_file/list_dir/glob/grep），权限层再硬拒写/编辑/shell，模型只能调研并输出计划
- Agent 循环：多轮工具调用、会话 JSONL 持久化、`AGENTS.md` 项目指令、权限确认
- 会话管理：`--continue`、`--list`、TUI 内 `/sessions`、`/session <id>` 切换
- 全局安装：`firm install` 复制到 `%USERPROFILE%\.firment\bin`、写用户 PATH、生成 PowerShell 补全
- 交互形态：TUI（ratatui，滚动/光标/滚轮/输入历史/模型选择器）+ `-p` 单次执行

## 快速开始

```powershell
cargo build --release
.\target\release\firm install      # 安装到 PATH，之后新开终端直接输入 firm
firm --doctor
firm                               # 交互式 TUI
firm -p "把 src/main.rs 里的 greet 函数改成打印 Hello"
```

升级新版本（从构建目录运行，避免覆盖正在运行的安装文件）：

```powershell
cargo build --release
.\target\release\firm update       # 用当前 release 覆盖已安装的 firm
```

### API 配置（只需一次）

首次运行会自动生成 `%APPDATA%\firment\config.toml`（Unix 为 `~/.config/firment/config.toml`，可用 `FIRMENT_CONFIG_DIR` 环境变量或 `--config` 指定位置）。默认 Provider 已指向 DeepSeek V4（`deepseek-v4-flash`）。

还没配 key 也能直接 `firm` 进 TUI（界面会提示），执行 `/apikey sk-xxx` 即可，不用先退出改文件。

API key 三种方式任选其一（之后都不用每次配置）：

1. TUI 里 `/apikey sk-xxx`，写入 `%APPDATA%\firment\auth.json`（推荐，与配置分开存放）
2. 命令行 `firm --set-key default=sk-xxx`
3. 环境变量 `DEEPSEEK_API_KEY`，或直接写进配置

```toml
[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"   # 或 deepseek-v4-pro

# 思考深度：off / low / medium / high / xhigh / max
thinking = "medium"
```

多 Provider 可追加配置，用 `--provider <名字>` 或 TUI 内 `/provider <名字>` 切换。

## 常用命令

- `firm install`：全局安装（写 PATH + PowerShell 补全）；`firm install --files-only` 只复制文件
- `firm update [<新exe路径>]`：覆盖已安装版本，默认源为当前运行的 release
- `firm`：交互式 TUI
- `firm -p "任务"`：单次执行
- `firm --plan -p "调研并给出实现计划"`：只读 Plan 模式单次执行
- `firm --continue`：恢复最近会话；`--continue <id>` 恢复指定会话
- `firm --thinking xhigh -p "任务"`：单次执行时指定思考深度（GPT 可用 xhigh，DeepSeek 可用 max）
- `firm --list`：列出会话
- `firm --doctor`：检查配置、Provider 连通性与安装状态
- `firm -y -p "任务"`：自动批准写/编辑/shell

TUI 内命令：

- `/plan`（不带参数则切换）/ `/plan on` / `/plan off`：进入/退出只读 Plan 模式（下一条消息起生效，状态栏显示 PLAN）
- `/agent`：显式切回普通 Agent 模式
- `/models`：从当前 Provider 拉取模型列表（像 opencode 一样，不用手改配置）
- `/model`（不带参数）或 `Ctrl+P`：打开可搜索模型选择器
- `/model <id>`：直接切换模型并保存
- `/sessions`：列出会话；`/session <id>`：切换会话
- `/provider <名字>`：切换 Provider 并保存为默认
- `/add-provider <名字> <openai|anthropic> <base_url> <模型>`：新增 Provider 并保存
- `/apikey [provider] <key>`：保存 API key（写入 auth.json，之后无需每次配置）
- `/thinking [off|low|medium|high|xhigh|max]`：切换思考深度（不带参数则循环切换）
- `/config`：查看当前配置和配置文件路径
- `/help`、`/clear`、`/quit`

TUI 键位：`↑/↓` 在输入框为空时浏览输入历史，非空时滚动对话；`PgUp/PgDn`、鼠标滚轮始终滚动；`Ctrl+P` 模型选择器；`←/→`、`Home/End`、`Ctrl+A/E` 移动输入光标；权限弹窗按 `y`/`n`/`a`。

## 结构

```text
crates/
  firment-core/   Provider 抽象、Agent 循环、会话、配置、权限、Tool trait
  firment-tools/  内置文件/搜索/shell 工具
  firment-tui/    ratatui 终端界面
  firment-cli/    clap 入口（bin: firm）+ 安装/更新/补全
```

后续各层（构建、UART、调试探头等）通过实现 `Tool` trait 注册到 `ToolRegistry` 接入同一内核。

## 质量门

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
