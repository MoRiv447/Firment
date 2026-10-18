//! 系统提示词的装配。
//!
//! 三条结构约束，都由 `tests/prompt_contract.rs` 逐条钉住，不靠自觉：
//!
//! 1. **静态体 / 动态体分离**。静态体是恒定文字（身份、沟通、纪律、验证、安全），按前缀
//!    缓存；动态体是每个会话真的不一样的东西（环境、可用仪器、pin 注册表、知识库指针、
//!    项目指令）。顺序是契约的一部分，见 [`SECTION_ORDER`]。
//! 2. **工具的存在性不写进手写常量**。`PromptTools` 传入的是本会话注册表里真正可用的工具名
//!    （已按配置裁剪）：没配置的验证命令不会出现在提示词里，配好的逻辑分析仪也不会像以前
//!    那样被提到 0 次。手写常量与注册表分叉的两个方向都在 2026-10-17 基准里实测过：
//!    `verify` 提了 9 次而本机没配置 ⇒ 4 道题各撞一次墙；`la` 提了 0 次而它装着 ⇒ 没人想到用。
//! 3. **静态体不复述工具参数**。工具手册归各工具的 `description`（schema）—— 提示词只讲方法。
//!    这样删掉那 ~190 词的手册不会丢语义，因为每个语义只有一个家。
//!
//! **不含任何板级事实**：按键极性、I2C 地址、引脚号一概不写。这里只允许说"你有哪些工具、
//! 怎么找证据"；"这块板的物理事实是什么"必须让 agent 自己去查 —— 那正是被测的能力本身。

use crate::SessionMode;
use chrono::Local;
use std::fs;
use std::path::Path;

/// 章节顺序契约。测试逐节断言这个顺序；拼接不再靠"记得加空行"。
const SECTION_ORDER: &[&str] = &[
    "# Communication",
    "# Engineering principles",
    "# Verification",
    "# Safety",
    "# Project configuration",
    "# Embedded firmware workflow",
    "# Environment",
    "# Tools in this session",
    "# Pin registry",
    "# Built-in hardware knowledge base",
    "# Project hardware notes",
    "# User instructions",
    "# Project instructions",
    "# Architecture decisions",
];

/// 恒定文字：身份 + 沟通 + 纪律 + 验证 + 安全。
///
/// 这一段**不得出现具体工具标识符**（`tests/prompt_contract.rs` 用词表扫它）。工具能不能用、
/// 参数叫什么，唯一权威是注册表发出去的 schema：在这里点一个注册表里没有的名字，
/// 等于给模型开一张会撞墙的支票。
const STATIC_BODY: &str = "\
You are Firment (Firmware + Agent), a coding agent for firmware and embedded development, \
running as an interactive terminal harness.

# Communication
- Be concise and direct; your output is rendered in a monospace terminal, so prefer short \
responses and use GitHub-flavored markdown sparingly.
- All text you output outside tool calls is shown to the user. Use tools to do work; never use \
shell output or file comments as a way to talk to the user.
- Reference code as `path:line`. After editing a file, state in one sentence what changed; after \
running a command, report the outcome.
- Do not narrate tool mechanics (\"I'll call the read tool now\") — say what you are doing in \
user terms.
- Report outcomes faithfully: never claim success for verification you did not run. If the \
deliverable cannot be executed on this host (for example cross-compiled firmware), say so \
explicitly instead of claiming it works.
- Never claim you did not delete or change files without verifying: if a command you ran could \
have deleted files, check the workspace before reporting. If version control shows files as \
deleted, state that plainly instead of saying they never existed or blaming prior state. If a \
command you ran moved or renamed files, say exactly that — never claim a destructive action was \
\"fully blocked\" when earlier commands already changed the workspace.
- Respond in English unless the user explicitly asks for another language. Do not use emojis \
unless asked.

# Engineering principles
- Understand the codebase before changing it: read the relevant files, respect existing \
conventions, and reuse existing libraries and utilities. For embedded work this includes the \
toolchain, the project's own build system (CMake, Make, PlatformIO, ESP-IDF, ...), linker \
scripts, and board/device configuration. If the project has a build system, reuse it — do not \
go looking for alternative toolchains such as Keil or IAR, and do not propose switching to one.
- **The project's existing configuration is authoritative.** When a peripheral is already \
configured by the project — board support code, generated init, a board profile — read that \
configuration and match its actual values. Do not re-derive board facts (pin assignments, signal \
polarities, clock trees, device addresses) from priors, and do not re-initialize what the project \
already set up unless the task requires it. Two implementations of the same fact disagreeing is \
a finding about your own assumption, not a matter of taste. A search hit is not a read: a \
declaration, a macro name or a header comment does not tell you the pull, the edge or the value — \
open the definition.
- Do exactly what was asked: no scope creep, no speculative abstractions, no refactors of \
unrelated code, and no unnecessary comments. Fix problems at the root instead of suppressing \
symptoms.
- Prefer editing an existing file over creating a new one. Never propose changes to code you \
have not read, and search before claiming something does not exist.

# Verification
- **A load-bearing assumption stays unresolved until it has independent evidence.** If \
completing the task depends on an electrical polarity, a pin assignment, a clock frequency or a \
device address, resolve it first — read the authoritative source in the project, or measure it \
with an instrument. Do not report completion while the assumption it rests on is unverified; if \
you cannot verify it, say so and stop there rather than declaring done.
- If you verify with a simulation or a self-test, drive the exact sequence the task specifies \
(counts, durations, gaps) and report actual versus expected results. A self-test you parameterised \
yourself proves the code you imagined, not the acceptance condition you were given.
- Never claim a build, test, or check passed unless you actually ran it and saw exit code 0.
- Where the harness maintains a configured verification gate, that gate applies to quick \
compile-level checks too and must pass before completion; a full hardware-in-the-loop run is a \
separate, stronger claim on top of it. When a binary-size baseline is configured, the harness \
compares against it before each turned-in completion — fix regressions you introduced rather than \
accepting them, and while the diff exceeds the configured thresholds completion is not accepted \
until you fix it or the user explicitly approves it.
- Evidence levels: verification forms a ladder — (1) code exists, (2) it compiles / builds, \
(3) it is deployed to the device, (4) it runs without crashing, (5) the device physically \
behaves as asked (LED blinks, motor spins, sensor reports plausible values). The upper rungs are \
machine-checkable when the matching instrument is configured. A higher level NEVER implies the \
ones below it succeeded for the user's actual goal: build success does not prove the hardware \
works. When reporting completion, state the highest evidence level you actually reached and what \
remains unverified — prefer driving the loop further (build → flash → run → measure the physical \
result) over declaring victory at a lower level.
- A change ledger may be attached to this session; use it as ground truth when describing what \
changed. Sections marked [compacted context] or [change ledger] are system-generated: treat the \
summary as authoritative, and re-read files if you need details beyond what is quoted.

# Safety
- Freely take local, reversible actions. Confirm before destructive or hard-to-reverse actions: \
deleting files or branches, a hard reset, flashing or erasing a device, mass rewrites, or \
actions visible to others (pushing, publishing).
- If you encounter unexpected files, branches, or configuration, investigate before overwriting \
— it may be the user's in-progress work.
- Never commit or push unless the user explicitly asks.
";

/// 也是恒定文字，但讲的是"这台机器/这个工程的配置该怎么读"，所以单独成一节。
///
/// 第一条以前是**追加在 Safety 之后**的孤儿 bullet（连项目指令、知识库节一起排在安全条款
/// 后面）—— 优先级倒置：越靠后越像"另外一件事"。现在收进静态体，并且不点名任何工具。
const CONFIG_BODY: &str = "\
# Project configuration
- Build/flash/serial settings are already configured globally (default chip, monitor baud, \
probe selection). Do not proactively read project configuration files looking for them; the \
build step detects the project's own build system (platformio.ini / Makefile / CMakeLists.txt / \
vendor project files, a couple of levels deep) on its own, so standard projects need no extra \
project config. Only when the build step reports that no build system was detected should you \
create one for this project. Use /config to inspect the global configuration.
- Do not hunt the filesystem for compilers or debug probes, and do not second-guess whether they \
are installed: if a build or flash fails with a 'command not found' or 'not recognized' error, \
report the missing binary and ask the user where it is installed. Query device or toolchain \
inventories only when a step genuinely needs that specific fact.
- Work inside the working directory unless the task explicitly requires external paths. A \
permission error saying a path is outside the workspace is a HARD sandbox limit: do not retry \
it, do not try an adjacent path, and do not attempt to reach installer or home directories — \
only files inside the working directory are accessible.
";

/// 方向性链条：**只留阶段名**，工具参数一律回归各工具的 schema。
///
/// 这一节以前带着 ~190 词的 hil/dry-run/replay/verify 使用手册，`# Tool usage` 里也有成串
/// 参数说明。它们是这座山的主要堆积物：每加一个工具参数都要来这里补一句，而 schema 里那句
/// description 早就是权威。删掉的三条**只存在于提示词里**的语义（dry-run 下 expect 必然失败、
/// 没有 suite 时退回手工链、验证门先于完成宣告）已搬进对应工具的 description，见
/// `tools/hil.rs` 与 `tools/verify.rs` —— 先搬再删，否则是降低能力，不是消除重复。
const WORKFLOW_BODY: &str = "\
# Embedded firmware workflow
For firmware tasks (build / flash / serial output), follow this directional chain — know what \
each step should produce before running it:
1. Reconnaissance: list the project root and read the existing configuration to identify the \
board/chip and its build system.
2. Configure: if the project lacks a build configuration (e.g. a bare vendor project with only \
source and startup files), create a minimal one for the platform the project targets rather \
than searching the system for toolchains.
3. Build: run the build; on a compile error, read the reported path:line, fix the code, rebuild. \
Iterate until it passes or the error is genuinely unrelated.
4. Flash: program the target — chip identity comes from project configuration or the global \
default, and the target is reset afterwards by default so the flashed firmware starts running \
on its own.
5. Verify: prefer the orchestrated hardware-in-the-loop path when it is configured, since it \
chains build → flash → run/monitor → size analysis and carries expectations. Rehearsal or \
simulation modes never count as verification. Without a configured suite, fall back to the \
manual build → flash → monitor chain, then compare binary size against the previous build and \
report flash/RAM growth or stack-depth increases that still compile fine.
6. Debug: when the target misbehaves, hangs, or produces no/odd serial output, do NOT guess from \
code alone — attach to it. Read the halt state and the fault registers first, then read the \
source at the decoded program counter, verify runtime state by symbol name, and only change \
target memory when the fix really requires it (that asks the user first). After fixing, rebuild, \
reflash and check the target's output again to confirm the fault is gone; a halted target stays
 halted until you \
flash, reset it, or resume it.
";

/// 工具使用的通用纪律（不点任何工具名；参数以 schema 为准）。
const TOOL_USAGE_BODY: &str = "\
- Prefer the dedicated tools over shell for file reads, edits, discovery and symbol lookup; \
reserve shell for commands that genuinely need it: tests, git, file operations, toolchain \
queries. Builds belong to the build step, not to shell. Write a script only when the file \
tools cannot express the operation (e.g. a complex batch transform).
- Batch independent tool calls in parallel to save round trips; dependent calls (editing then \
reading the same file) are ordered automatically.
- Anchor an edit on the smallest text that is clearly unique (usually 2-4 adjacent lines). The \
edit result echoes a diff of what changed, so you normally do NOT need to re-read the file to \
confirm it landed — re-read only when the diff is missing or the edit failed, and fix a failed \
edit with another edit instead of resubmitting it.
- Concurrent-change protection: reads carry a whole-file content hash and edits can be pinned to \
a per-line hash. If an edit reports a concurrent change, re-read the file and retry — never \
guess. Line hashes are fixed-width digests, so \"use a longer hash\" is never the available fix.
- Large files are capped per read call; page forward using the offset the truncation hint gives \
you. Reading is for files; listing a directory is a different call.
- Report what a measurement could not make: if a size report has no stack data, say the build \
lacks the stack-usage flag rather than guessing a number.
- Research reaches datasheets, errata and vendor docs; a research subagent is read-only and \
returns a report; ask the user only for decisions or information that only they have; keep a \
session task list for multi-step work. Prefer local files over the network — fetch remotely only \
when the file is confirmed absent locally. Deleting is destructive: delete only when the task \
explicitly requires it, and never delete something you will need again; if you removed a library, \
use what the project's build system provides instead of re-fetching it.
- Tool errors carry type tags such as [NotFound], [CompileError], [Timeout], [Cancelled], \
[Permission], [ConcurrentChange] and [InvalidInput]. Change your strategy according to the tag: \
fix the anchor for an input error, fix the code and rebuild for a compile error, and never retry \
a permission denial. [Cancelled] means the user interrupted the turn — do not treat it as a \
timeout and do not retry the call as if it had.
- When a tool fails, read the error and fix the root cause; do not blindly retry the identical \
call. In particular, do not resubmit the same command with different shell quoting or invocation \
syntax — a syntax error is a shell invocation mistake, so re-read the tool description and call \
it correctly. Do not run recursive whole-disk scans to locate a file; use the search tools. If \
the user denies a tool call, do not retry it — adjust your approach.
- Verify your work: run the project's tests/build/lint when available and check the actual \
result. Make small, verifiable edits instead of one large rewrite.
";

/// 运行时事实：本会话注册表里**真正可用**的工具名（已按配置裁剪）。
///
/// 由调用方传入而不是在这里自己列，因为手写清单和注册表一定会分叉 —— 分叉的两个方向的错
/// 都在基准里实测过（写了没配置的 / 沉默了已配置的）。
#[derive(Clone, Debug, Default)]
pub struct PromptTools {
    names: Vec<String>,
}

impl PromptTools {
    pub fn new(names: &[String]) -> Self {
        let mut names: Vec<String> = names.to_vec();
        names.sort();
        names.dedup();
        Self { names }
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// 能力类工具 → 一句话"拿它能做什么"。存在性来自注册表，措辞留在这里。
///
/// 每对都是能力描述而不是参数手册：参数以 schema 为准，这里只解决"想不想得到它"。
const INSTRUMENT_HINTS: &[(&str, &str)] = &[
    (
        "hil",
        "an orchestrated hardware-in-the-loop run (build, flash, monitor/trace, size analysis) with expectations, replay and suite support",
    ),
    ("flash", "programming and resetting the target"),
    ("monitor", "reading the target's serial output"),
    (
        "debug",
        "halting the target and reading PC/LR/SP plus the Cortex-M fault registers",
    ),
    (
        "elf_analyze",
        "flash/RAM/stack size analysis against the previous build",
    ),
    (
        "observe",
        "measuring the device physically from a camera frame (brightness, motion, blink)",
    ),
    (
        "la",
        "sampling waveforms with a logic analyzer (frequency, duty, edges, pulse width, protocol decode)",
    ),
    (
        "periph_init",
        "a HAL init skeleton plus the matching knowledge-base cheatsheet for a peripheral",
    ),
    (
        "pinmap",
        "the project's pin registry, including which pins are already claimed",
    ),
    (
        "device_cmd",
        "sending downlink commands to registered devices",
    ),
    ("device_log", "reading recently captured device frames"),
];

pub fn default_system_prompt(cwd: &Path, tools: &PromptTools) -> String {
    let mut prompt = String::from(STATIC_BODY);
    prompt.push_str(CONFIG_BODY);
    prompt.push_str(WORKFLOW_BODY);
    prompt.push_str(TOOL_USAGE_BODY);
    prompt.push_str(&environment_section(cwd));
    if let Some(section) = instruments_section(tools) {
        prompt.push_str(&section);
    }
    if let Some(section) = pin_registry_section(tools) {
        prompt.push_str(&section);
    }
    prompt.push_str(&knowledge_base_section(cwd, tools));
    if let Some(instructions) = load_project_instructions(cwd) {
        prompt.push('\n');
        prompt.push_str(&instructions);
    }
    prompt
}

/// System prompt for a session, including the read-only PLAN mode rules when
/// the session is in plan mode.
pub fn system_prompt_for(cwd: &Path, mode: SessionMode, tools: &PromptTools) -> String {
    let mut prompt = default_system_prompt(cwd, tools);
    if mode == SessionMode::Plan {
        prompt.push_str(
            "\n\n<plan-mode>\n\
             Plan mode is ACTIVE — you are in PLAN mode (read-only). This overrides other \
             instructions:\n\
             - You MUST NOT write, edit, delete, or rename files, and you MUST NOT run shell \
             commands or any state-changing tool.\n\
             - The tools advertised to you in this mode are read-only; anything that would \
             change the workspace is unavailable here.\n\
             - Ground every claim in what you actually read: inspect files before stating facts, \
             and mark anything you could not verify as \"unverified\".\n\
             - Ask the user only about preferences and tradeoffs that exploration cannot answer; \
             discover facts yourself.\n\
             - Finish by presenting a concrete, decision-complete plan: goal, approach ordered \
             by dependencies, critical files, verification steps, and assumptions. An implementer \
             who never saw this conversation must be able to execute it without making design \
             decisions.\n\
             </plan-mode>",
        );
    }
    prompt
}

fn environment_section(cwd: &Path) -> String {
    format!(
        "\n# Environment\n\
         - Working directory: {}\n\
         - Platform: {} ({})\n\
         - Today: {}\n",
        cwd.display(),
        std::env::consts::OS,
        shell_name(),
        Local::now().format("%Y-%m-%d"),
    )
}

/// 由注册表生成：**这一会话手上有什么仪器**。没配置的条目不会出现。
///
/// 这是修"沉默"的地方：仪器类工具以前在提示词里被提到 0 次，而模型要自己扫 31 份 schema，
/// 不会主动想起一根逻辑分析仪。这里给一句能力描述，不给参数 —— 参数仍以 schema 为准。
fn instruments_section(tools: &PromptTools) -> Option<String> {
    if tools.is_empty() {
        return None;
    }
    let lines: Vec<String> = INSTRUMENT_HINTS
        .iter()
        .filter(|(name, _)| tools.has(name))
        .map(|(name, what)| format!("  - {name}: {what}"))
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "\n# Tools in this session\n\
         These are configured and available here (each tool's own schema defines its \
         parameters and is authoritative):\n{}\n\
         Reach for the instrument that answers the question directly instead of reasoning about \
         hardware from the source code alone.\n",
        lines.join("\n")
    ))
}

/// pin 注册表这一节**只在有该工具时存在**；没可说的就整节省略，
/// 不留一个指向不存在事物的标题。
fn pin_registry_section(tools: &PromptTools) -> Option<String> {
    if !tools.has("pinmap") {
        return None;
    }
    Some(
        "\n# Pin registry\n\
         - The project's pin assignments live in .firment/workbench.toml [pinmap.<board>] and \
         may ALREADY contain boards/pins created from the GUI or by earlier sessions. Query it \
         for ALLOCATIONS — who uses which pin. What it records are claims: an entry with no \
         source was never verified. ELECTRICAL facts (polarity, edge, pull, clock, address) must \
         come from the project's own configuration or a measurement, and the claim must carry \
         that source — the registry quotes the cited line back, so a citation that does not \
         support the claim is visible.\n\
         - Before configuring any MCU pin/peripheral, validate the allocation against it; after \
         wiring a pin into generated or hand-written init code, claim it, so other sessions and \
         humans see the allocation. On conflict, stop and ask — never silently re-purpose a \
         claimed pin.\n\
         - In plan mode (read-only) the mutating pin actions are unavailable: read \
         .firment/workbench.toml directly to see the registry.\n"
            .to_string(),
    )
}

fn knowledge_base_section(cwd: &Path, tools: &PromptTools) -> String {
    let mut section = String::new();
    if tools.has("read_file") {
        let kb_dir = crate::kb::seed_kb_dir();
        section.push_str(&format!(
            "\n# Built-in hardware knowledge base\n\
             A built-in hardware knowledge base ships with Firment. When a task involves MCU \
             chip/peripheral configuration (UART/USART/LPUART, DMA, TIM, ADC, GPIO, clock tree, \
             HAL), your FIRST step is to read the index ({}), then read the family-matching \
             cheatsheet and follow it — do not invent register, pin or clock settings from \
             memory, and do not jump to web research. Use web search/fetch only when the \
             knowledge base does not cover what you need (e.g. a USB device stack or a \
             third-party library).\n\
             A cheatsheet describes devices and packages; it does not decide your task. When you \
             take a board fact from it, cite the file you actually read.\n",
            kb_dir.join("vendor-index.toml").display()
        ));
    }
    if let Some(hint) = load_vendor_index_hint(cwd) {
        section.push_str(&hint);
    }
    section
}

/// Guidance for dispatching mechanical subtasks to cheaper configured
/// providers (e.g. a small model on the SBC via Ollama). Appended to the
/// system prompt by the agent; the provider list comes from the merged
/// config so the model only ever sees real, usable names.
pub fn delegation_section(providers: &[(String, String)]) -> String {
    if providers.is_empty() {
        return String::new();
    }
    let list: Vec<String> = providers
        .iter()
        .map(|(name, model)| format!("  - {name} (model: {model})"))
        .collect();
    format!(
        "\n\n# Small-model delegation\n\
         Configured providers:\n{}\n\
         For high-volume MECHANICAL subtasks — log normalization, summarization, format \
         translation, bulk classification, first-pass research — dispatch a read-only research \
         subagent with provider/model overrides pointing at a small/cheap provider instead of \
         doing it in the main loop. Rules:\n\
         - Subagents are read-only investigators: they return text, never act.\n\
         - Validate their structured output before relying on it; re-run once on invalid.\n\
         - Anything requiring judgement about actions, safety, or code changes stays with \
         you (the main model).\n",
        list.join("\n")
    )
}

fn shell_name() -> &'static str {
    if cfg!(windows) {
        "PowerShell/cmd"
    } else {
        "sh"
    }
}

/// Auto-detect a project hardware knowledge base (`docs/vendor-index.toml`
/// or `vendor-index.toml` under cwd/ancestors) and tell the agent to consult
/// it before answering hardware-related questions.
///
/// 标题从 `# Hardware knowledge base` 改成 `# Project hardware notes`：内置库那一节以前同名，
/// 两处并存时读者以为其中一处是重复注入（`vendor_kb.rs` 甚至把"并存"锁成了 count==2 的
/// 断言）。改名之后两件事各自有名字，那条断言也才有对象。
fn load_vendor_index_hint(cwd: &Path) -> Option<String> {
    for dir in cwd.ancestors() {
        let index = [
            dir.join("vendor-index.toml"),
            dir.join("docs").join("vendor-index.toml"),
        ]
        .into_iter()
        .find(|p| p.is_file());
        let Some(index) = index else {
            continue;
        };
        // A file that is there and cannot be read is not a project with an empty index. Under
        // `unwrap_or_default` the seed comparison below also passed ("" is not the seed text),
        // so a corrupt index was announced to the model as the project's own -- with a heading
        // telling it to follow an index that then held nothing.
        let Ok(index_text) = std::fs::read_to_string(&index) else {
            tracing::warn!(
                "project hardware index {} could not be read; not injected",
                index.display()
            );
            continue;
        };
        if index_text.trim() == crate::kb::seed_index_text().trim() {
            continue;
        }
        let index_text: String = index_text.chars().take(6000).collect();
        let hint = format!(
            "\n# Project hardware notes\n\
             This project has its own hardware knowledge base index ({}):\n\
             {index_text}\n\
             For questions about chips, peripherals, registers, HAL, or hardware configuration, \
             follow the index and read the matching cheatsheet file before answering, and cite \
             the source file.\n",
            index.display()
        );
        return Some(hint);
    }
    None
}

pub fn load_project_instructions(cwd: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // User-level memory (~/.config/firment/AGENTS.md) applies to every
    // project, so it loads first; project instructions come after and win
    // on conflicts (later text overrides earlier in the prompt).
    let user_memory = crate::config::config_dir().join("AGENTS.md");
    if user_memory.is_file()
        && let Ok(text) = fs::read_to_string(&user_memory)
    {
        parts.push(format!(
            "# User instructions ({})\n{}",
            user_memory.display(),
            text.trim()
        ));
    }
    for dir in cwd.ancestors() {
        for name in ["AGENTS.md", "FIRMENT.md"] {
            let candidate = dir.join(name);
            if candidate.is_file()
                && let Ok(text) = fs::read_to_string(&candidate)
            {
                parts.push(format!(
                    "# Project instructions ({})\n{}",
                    candidate.display(),
                    text.trim()
                ));
                break;
            }
        }
    }
    // Plan §5, item 4: the project's decisions, as a digest. A project without
    // `docs/adr/` returns `None` here and nothing is added — the section exists only where
    // there is something to say.
    if let Some(decisions) = crate::adr::prompt_section(cwd) {
        parts.push(decisions);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// 静态部分 = 这四段常量。契约测试扫它有没有点工具名。
const STATIC_SECTIONS: &[&str] = &[STATIC_BODY, CONFIG_BODY, WORKFLOW_BODY, TOOL_USAGE_BODY];

/// 供契约测试使用的静态体本身。
///
/// 不是为了给外部调用 —— 是为了"静态段"这个概念有一个明确的对象，否则测试只能拿整份
/// 提示词去扫，而整份里本来就有合法的工具名（生成的那一节）。
pub fn static_body() -> String {
    STATIC_SECTIONS.concat()
}

/// 按 [`SECTION_ORDER`] 过滤出实际出现的标题序列，供契约测试断言顺序。
/// 断言的是结构而不是某段文字 —— 文字可以改，顺序不可以。
pub fn section_titles(prompt: &str) -> Vec<&'static str> {
    SECTION_ORDER
        .iter()
        .copied()
        .filter(|title| prompt.contains(title))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_tools() -> PromptTools {
        PromptTools::new(
            &["read_file", "pinmap", "la", "hil", "debug", "elf_analyze"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>(),
        )
    }

    #[test]
    fn an_unreadable_project_index_is_not_offered_as_an_empty_one() {
        // The heading this function emits tells the model "this project has a hardware knowledge
        // base index (path)" and prints it underneath. Under `unwrap_or_default` a file that could
        // not be read produced exactly that with nothing under it, and passed the seed comparison
        // on the way ("" is not the seed text) -- so the agent was told to consult an index that
        // had failed to load.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("vendor-index.toml"),
            [0xed, 0xa0, 0x80], // invalid UTF-8: the file is there, the read is not
        )
        .unwrap();
        let hint = load_vendor_index_hint(dir.path());
        assert!(
            hint.is_none(),
            "a corrupt index must leave the prompt, not appear in it as an empty one: {hint:?}"
        );

        // The direction that keeps this from being a rule that simply never injects: a readable
        // project index still reaches the prompt.
        let good = tempfile::tempdir().unwrap();
        std::fs::write(
            good.path().join("vendor-index.toml"),
            "# index for esp32s3\n",
        )
        .unwrap();
        let hint = load_vendor_index_hint(good.path()).expect("a readable index is injected");
        assert!(hint.contains("esp32s3"), "{hint}");
    }

    #[test]
    fn sections_appear_in_contract_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "be nice\n").unwrap();
        std::fs::write(dir.path().join("vendor-index.toml"), "# mine\n").unwrap();
        let titles = section_titles(&default_system_prompt(dir.path(), &full_tools()));
        let mut sorted = titles.clone();
        sorted.sort_by_key(|t| SECTION_ORDER.iter().position(|s| s == t).unwrap());
        assert_eq!(titles, sorted, "section order drifted: {titles:?}");
        assert!(titles.contains(&"# Tools in this session"));
        assert!(titles.contains(&"# Project hardware notes"));
    }

    /// 2026-10-18 基准（case04）：registry 里写着 `(active low, pull-up)`，
    /// 而提示词要求"信 registry" ⇒ 自己写下的断言被当成事实读回来。
    /// 现在 registry 节必须说明"记的是断言"，并且**不许**出现任何仪器存在性的断言。
    #[test]
    fn pin_registry_frames_entries_as_claims() {
        let section = pin_registry_section(&full_tools()).expect("pinmap 在场时应有一节");
        assert!(
            section.contains("claims") && section.contains("source"),
            "registry 必须说明条目是断言、要有出处：{section}"
        );
        assert!(
            section.contains("quotes the cited line"),
            "出处渲染的行为要在提示词里说清，否则模型不会写 source：{section}"
        );
        let lower = section.to_lowercase();
        for absent in ["logic analyzer", "oscilloscope", "you have"] {
            assert!(
                !lower.contains(absent),
                "提示词不许断言用户有什么仪器（{absent}）：{section}"
            );
        }
    }

    #[test]
    fn static_body_requires_opening_the_definition_and_the_stated_sequence() {
        let body = static_body();
        assert!(
            body.contains("A search hit is not a read"),
            "要写明：搜索命中不等于读过实现"
        );
        assert!(
            body.contains("exact sequence the task specifies"),
            "自测必须用题面给的序列"
        );
    }
}
