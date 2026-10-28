//! 探针后端：把 `probe-rs` 从"外部命令行"换成"链接进来的库"。
//!
//! 为什么值得换（不是风格问题）：
//!
//! 1. **一次事务里读多个寄存器**。今天 `debug mem` 是 shell 出去一次，读 6 个字是**一个时刻**；
//!    但"读 MODER、读 PUPDR、读 IDR"是三次调用、三个时刻 —— case04 的误读就是在这种缝里发生的。
//! 2. **错误是类型**。`probe_rs::Error` 分得清"没有探针""芯片不匹配""Flash 被保护"，
//!    而 CLI 只有一句 `exited 1` 加一段 stderr，靠关键词猜。
//! 3. **attach 一次，调用多次**。CLI 每次调用都重连（~1s）并丢掉 halt 状态。
//!
//! 这一层刻意做得很薄：**只包我们已经验证过的操作**（读字、写字、halt、resume），
//! 不追求把 probe-rs 的 API 全暴露出来。CLI 路径保留为 fallback —— 库初始化失败时
//! （老探针、缺驱动、Windows 上的 USB 抢占）不能让一个本来能用的功能变砖。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::tools::util::ProbeLease;
use probe_rs::{MemoryInterface, Session, SessionConfig};

/// 会话闲置多久后释放。拿太久会让 PulseView / `probe-rs` CLI 抢不到探针；
/// 拿太短则每次调用都重连。30 秒覆盖"连续几条 `debug` 调用"这一实际场景。
const IDLE_RELEASE: Duration = Duration::from_secs(30);

struct Cached {
    chip: String,
    probe: Option<String>,
    session: Session,
    last_used: Instant,
}

static SESSION: Mutex<Option<Cached>> = Mutex::new(None);

/// 错误分三类，调用方据此决定要不要回退到 CLI。
#[derive(Debug)]
pub enum ProbeError {
    /// 库拿不到探针/芯片：退回 CLI 是合理的（CLI 可能配了别的探针选择方式）。
    Unavailable(String),
    /// 拿到了，操作失败：这是真错误，回退只会把同一个错误再报一次。
    Failed(String),
}

impl ProbeError {
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(m) | Self::Failed(m) => m,
        }
    }
}

fn open(chip: &str, probe: Option<&str>) -> Result<Session, ProbeError> {
    // `auto_attach` 会选第一个可用探针；显式 `probe` 目前只用于日志与缓存键，
    // 真正的选择等 probe-rs 的 `Probe::list_all` 接进来再做（见 TODO）。
    // TODO(probe-select): 按 serial 选探针，多探针台架上别拿错板子。
    let _ = probe;
    Session::auto_attach(chip, SessionConfig::default()).map_err(|e| {
        ProbeError::Unavailable(format!(
            "[NoProbe] probe-rs (library) could not attach to '{chip}': {e}"
        ))
    })
}

/// 拿到会话：命中缓存且未超时就复用，否则重连。
fn with_session<T>(
    chip: &str,
    probe: Option<&str>,
    f: impl FnOnce(&mut Session) -> Result<T, ProbeError>,
) -> Result<T, ProbeError> {
    let mut slot = SESSION
        .lock()
        .map_err(|_| ProbeError::Failed("[Io] probe session lock poisoned".into()))?;

    // 超时或换了芯片：先释放旧的，再开新的。
    let stale = match slot.as_ref() {
        Some(c) => {
            c.chip != chip || c.probe.as_deref() != probe || c.last_used.elapsed() > IDLE_RELEASE
        }
        None => true,
    };
    if stale {
        *slot = None;
        let session = open(chip, probe)?;
        *slot = Some(Cached {
            chip: chip.to_string(),
            probe: probe.map(|p| p.to_string()),
            session,
            last_used: Instant::now(),
        });
    }

    let cached = slot.as_mut().expect("just filled");
    let out = f(&mut cached.session);
    cached.last_used = Instant::now();
    out
}

fn core(session: &mut Session) -> Result<probe_rs::Core<'_>, ProbeError> {
    session
        .core(0)
        .map_err(|e| ProbeError::Failed(format!("[Io] probe-rs could not select core 0: {e}")))
}

/// 读 `n` 个 32 位字。
pub fn read_words(
    chip: &str,
    probe: Option<&str>,
    addr: u64,
    n: usize,
) -> Result<Vec<u32>, ProbeError> {
    let _lease = ProbeLease::acquire("a library debug read").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        let mut words = vec![0u32; n];
        core.read_32(addr, &mut words)
            .map_err(|e| ProbeError::Failed(format!("[Io] read {n} words at {addr:#010x}: {e}")))?;
        Ok(words)
    })
}

/// 写一个 32 位字。**调用方负责知道自己在写什么**（见 `debug` 的 GPIO 警告）。
pub fn write_word(
    chip: &str,
    probe: Option<&str>,
    addr: u64,
    value: u32,
) -> Result<(), ProbeError> {
    let _lease = ProbeLease::acquire("a library debug write").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        core.write_word_32(addr, value)
            .map_err(|e| ProbeError::Failed(format!("[Io] write {value:#x} to {addr:#010x}: {e}")))
    })
}

// halt / resume 先不包：它们要和 flash/run 一起切过来才有意义，
// 单独切换会让"CLI 烧录 + 库 halt"这种混搭状态更难解释。

/// 采集 RTT 输出，直到 `duration` 到点或 `stop` 被置位。
///
/// RTT 是"库能做到、CLI 做不到"的那件事：`probe-rs run` 只在它自己的进程里
/// 打印，我们拿不到结构化输出，也没法在同一个会话里接着读寄存器。
///
/// 返回写出的字节数。没有控制块（固件没接 RTT）时返回 `Failed`，由调用方
/// 决定回退 CLI —— 那可能是 semihosting 的工程。
pub fn rtt_stream(
    chip: &str,
    probe: Option<&str>,
    duration: Option<Duration>,
    stop: &std::sync::atomic::AtomicBool,
    mut sink: impl FnMut(&[u8]),
) -> Result<usize, ProbeError> {
    use std::sync::atomic::Ordering;

    let _lease = ProbeLease::acquire("a library RTT read").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        let mut rtt = probe_rs::rtt::Rtt::attach(&mut core).map_err(|e| {
            ProbeError::Failed(format!(
                "[NotFound] no RTT control block found — the firmware may not initialise RTT \
                 (or uses semihosting): {e}"
            ))
        })?;
        let deadline = duration.map(|d| Instant::now() + d);
        let mut buf = [0u8; 1024];
        let mut total = 0usize;
        while deadline.is_none_or(|d| Instant::now() < d) && !stop.load(Ordering::Relaxed) {
            let mut got = false;
            for i in 0..rtt.up_channels().len() {
                let Some(channel) = rtt.up_channel(i) else {
                    continue;
                };
                // 空通道返回 Ok(0)，不是错误；真正的错误（目标停了）留给下一轮。
                if let Ok(n) = channel.read(&mut core, &mut buf)
                    && n > 0
                {
                    sink(&buf[..n]);
                    total += n;
                    got = true;
                }
            }
            if !got {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Ok(total)
    })
}

/// 目标侧的 TPIU / ITM 配置。
///
/// probe-rs 把 `configure_tpiu` 留成了 crate 私有（只给 CLI 用），而 `enable_swo`
/// 只切探针侧的引脚模式。后果是：库路径能"成功打开 SWO"却收不到任何字节 ——
/// 2026-10-20 用 `fixtures/itm-probe-fw` 实测到的就是这一条。
///
/// 这些寄存器都是**内存映射**的（TPIU 0xE0040000、ITM 0xE0000000），所以不必碰
/// 私有的 Coresight 组件 API，直接用 `MemoryInterface` 写就行。取值照 ARMv7-M
/// 与 probe-rs 的实现：1 位端口、NRZ 异步、prescaler = clk/baud - 1、FFCR=0x100。
fn configure_itm_swo(
    core: &mut probe_rs::Core<'_>,
    tpiu_hz: u32,
    baud: u32,
) -> Result<(), ProbeError> {
    use probe_rs::MemoryInterface;

    const ITM_LAR: u64 = 0xE000_0FB0;
    const ITM_TCR: u64 = 0xE000_0E80;
    const TPIU_CSPSR: u64 = 0xE004_0004;
    const TPIU_ACPR: u64 = 0xE004_0010;
    const TPIU_SPPR: u64 = 0xE004_00F0;
    const TPIU_FFCR: u64 = 0xE004_0304;

    let write = |core: &mut probe_rs::Core<'_>, addr: u64, value: u32| {
        core.write_word_32(addr, value)
            .map_err(|e| ProbeError::Failed(format!("[Io] write {addr:#010x} <- {value:#x}: {e}")))
    };

    // ITM 解锁（不写这个，TCR/TER 的写入会被忽略）。
    write(core, ITM_LAR, 0xC5AC_CE55)?;
    // TCR: ITMENA（bit0）。TraceBusID 保持 0 —— 单源、无全局时间戳。
    write(core, ITM_TCR, 0x1)?;

    // TPIU：1 位端口、异步 NRZ、prescaler、格式化器透传。
    write(core, TPIU_CSPSR, 0x1)?;
    let prescaler = tpiu_hz / baud.max(1);
    write(core, TPIU_ACPR, prescaler.saturating_sub(1))?;
    write(core, TPIU_SPPR, 0x2)?;
    write(core, TPIU_FFCR, 0x100)?;
    Ok(())
}

/// 一次 SWO 采集的结果。
pub struct SwoTrace {
    pub lines: Vec<crate::itm_decode::ItmLine>,
    pub stats: crate::itm_decode::ItmStats,
    pub bytes: usize,
}

/// 采 `duration` 的 SWO 并用本地 ITM 解码器解出 stimulus 输出。
///
/// 传输在库里（`SwoAccess`），解码在 `itm_decode`（我们自己的）。
/// 这与 CLI 的 `probe-rs itm swo` 是同一件事，区别是字节回到我们手里，
/// 能直接进工具输出、也能被 `hil` 的 trace 步骤复用。
pub fn swo_trace(
    chip: &str,
    probe: Option<&str>,
    duration: Duration,
    tpiu_hz: u32,
    baud: u32,
) -> Result<SwoTrace, ProbeError> {
    use probe_rs::architecture::arm::swo::SwoConfig;

    let _lease = ProbeLease::acquire("a library SWO trace").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let config = SwoConfig::new(tpiu_hz).set_baud(baud);

        // `get_arm_interface()` 与 `core()` 都要可变借用 session，所以分开取用完就放。
        {
            let interface = session
                .get_arm_interface()
                .map_err(|e| ProbeError::Failed(format!("[Io] no ARM interface for SWO: {e}")))?;
            interface
                .enable_swo(&config)
                .map_err(|e| ProbeError::Failed(format!("[Io] could not enable SWO: {e}")))?;
        }

        // 探针侧配好后，配目标侧（TPIU/ITM）。两边都要配，只配一边就收不到字节。
        {
            let mut trace_core = core(session)?;
            configure_itm_swo(&mut trace_core, tpiu_hz, baud)?;
        }

        let interface = session
            .get_arm_interface()
            .map_err(|e| ProbeError::Failed(format!("[Io] no ARM interface for SWO: {e}")))?;

        let mut decoder = crate::itm_decode::ItmDecoder::new();
        let mut lines = Vec::new();
        let mut bytes = 0usize;
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            // 50 ms 一片：既不会把 CPU 占死，也不会让缓冲溢出。
            match interface.read_swo_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    bytes += chunk.len();
                    lines.extend(decoder.push(&chunk));
                }
                // 读失败不是"没有数据"，但 SWO 的读失败通常是探针端缓冲问题；
                // 记下来，别把它当成空采集。
                Err(e) => {
                    let _ = interface.disable_swo();
                    return Err(ProbeError::Failed(format!("[Io] SWO read failed: {e}")));
                }
            }
        }
        let _ = interface.disable_swo();
        lines.extend(decoder.flush());
        Ok(SwoTrace {
            lines,
            stats: decoder.stats(),
            bytes,
        })
    })
}

/// 一帧栈回溯。只保留工具层要显示的东西，不把 `probe_rs_debug` 的类型漏出去。
#[derive(Debug, Clone)]
pub struct BacktraceFrame {
    pub pc: u64,
    pub function: String,
    pub location: Option<String>,
}

/// DWARF 栈回溯。
///
/// 库 `probe-rs-debug` 才是这套东西的实现者 —— `probe-rs-debugger` 是空壳，
/// `probe-rs-tools` 是纯二进制。这里只做"停核 → 取寄存器 → unwind → 摊平成字符串"。
///
/// `elf` 必须带 DWARF（`-g`）。没有调试信息时 unwind 会退化成只有 pc 的帧，
/// 调用方据此提示重建，而不是报错。
pub fn backtrace(
    chip: &str,
    probe: Option<&str>,
    elf: &std::path::Path,
    max_frames: usize,
) -> Result<Vec<BacktraceFrame>, ProbeError> {
    use probe_rs::InstructionSet;
    use probe_rs_debug::DebugInfo;
    use probe_rs_debug::registers::DebugRegisters;

    let _lease = ProbeLease::acquire("a library backtrace").map_err(ProbeError::Failed)?;
    let info = DebugInfo::from_file(elf).map_err(|e| {
        ProbeError::Failed(format!(
            "[InvalidInput] cannot read debug info from {}: {e}",
            elf.display()
        ))
    })?;

    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        let _ = core.halt(Duration::from_millis(500));
        let regs = DebugRegisters::from_core(&mut core);
        // 按核类型取对应的异常帧解析器（Armv6/7/8、RISC-V、Xtensa 都有）。
        let handler = probe_rs_debug::exception_handler_for_core(core.core_type());
        let frames = info
            .unwind(
                &mut core,
                regs,
                handler.as_ref(),
                Some(InstructionSet::Thumb2),
                max_frames,
            )
            .map_err(|e| ProbeError::Failed(format!("[Io] unwind failed: {e}")))?;

        Ok(frames
            .iter()
            .map(|f| BacktraceFrame {
                pc: match f.pc {
                    probe_rs::RegisterValue::U32(v) => u64::from(v),
                    probe_rs::RegisterValue::U64(v) => v,
                    probe_rs::RegisterValue::U128(v) => v as u64,
                },
                function: f.function_name.clone(),
                location: f
                    .source_location
                    .as_ref()
                    .map(|l| format!("{}:{}", l.path.to_string_lossy(), l.line.unwrap_or(0))),
            })
            .collect())
    })
}

/// 跑到某个地址并停在那里。
///
/// 与 CLI 的 `-c break -c "break *addr" -c c -c "info reg"` 对应，但：
/// 断点是硬件比较器、命中由 `status()` 报出来，不是靠解析文本；
/// 而且**无论成功与否都会清掉断点**，否则下一次 `run` 会莫名其妙停住。
pub fn run_to_breakpoint(
    chip: &str,
    probe: Option<&str>,
    addr: u64,
    timeout: Duration,
) -> Result<Registers, ProbeError> {
    let _lease = ProbeLease::acquire("a library breakpoint run").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        // 设断点要求核已停：先停一下，再装比较器。
        let _ = core.halt(Duration::from_millis(500));
        core.set_hw_breakpoint(addr).map_err(|e| {
            ProbeError::Failed(format!(
                "[Io] could not set a hardware breakpoint at {addr:#010x}: {e}"
            ))
        })?;

        // 收尾：清断点在**所有**返回路径上都要发生。
        let cleanup = |core: &mut probe_rs::Core<'_>| {
            let _ = core.clear_hw_breakpoint(addr);
        };

        if let Err(e) = core.run() {
            cleanup(&mut core);
            return Err(ProbeError::Failed(format!(
                "[Io] resume to breakpoint: {e}"
            )));
        }

        let deadline = Instant::now() + timeout;
        loop {
            match core.status() {
                Ok(probe_rs::CoreStatus::Halted(reason)) => {
                    let regs = snapshot(&mut core);
                    cleanup(&mut core);
                    return match regs {
                        Ok(r) => {
                            if r.pc != addr && r.pc != addr + 2 {
                                // 停在了别的原因上（异常、看门狗）——别假装是断点。
                                return Err(ProbeError::Failed(format!(
                                    "[Io] the core stopped at {:#010x} ({reason:?}), not at the \
                                     breakpoint {addr:#010x}",
                                    r.pc
                                )));
                            }
                            Ok(r)
                        }
                        Err(e) => Err(e),
                    };
                }
                Ok(probe_rs::CoreStatus::LockedUp) => {
                    cleanup(&mut core);
                    return Err(ProbeError::Failed(
                        "[Io] the core locked up before reaching the breakpoint".to_string(),
                    ));
                }
                Err(e) => {
                    cleanup(&mut core);
                    return Err(ProbeError::Failed(format!("[Io] status: {e}")));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                cleanup(&mut core);
                return Err(ProbeError::Failed(format!(
                    "[Timeout] the breakpoint at {addr:#010x} was not hit within {} ms; the \
                     target is left running",
                    timeout.as_millis()
                )));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })
}

/// 核的状态操作。三个都用 `Core` 的直接方法，不需要解析任何文本。
#[derive(Debug, Clone, Copy)]
pub enum CoreOp {
    Halt,
    Resume,
    Step,
}

/// 停核 / 放开 / 单步。
///
/// 与 CLI 的语义差异要写下来：CLI 的 `continue` 会在**它自己退出时**把目标再停住
/// （probe-rs 的 REPL 是 attach 式的）；这里的会话是常驻的，`Resume` 之后目标一直跑，
/// 直到我们停它、或者会话闲置 30 秒被释放。调用方的输出要照这个事实写。
pub fn core_op(chip: &str, probe: Option<&str>, op: CoreOp) -> Result<(), ProbeError> {
    let _lease = ProbeLease::acquire("a library core operation").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        match op {
            CoreOp::Halt => core
                .halt(Duration::from_millis(500))
                .map(|_| ())
                .map_err(|e| ProbeError::Failed(format!("[Io] halt: {e}"))),
            CoreOp::Resume => core
                .run()
                .map_err(|e| ProbeError::Failed(format!("[Io] resume: {e}"))),
            CoreOp::Step => core
                .step()
                .map(|_| ())
                .map_err(|e| ProbeError::Failed(format!("[Io] step: {e}"))),
        }
    })
}

/// 一个核的寄存器快照。
///
/// 为什么用结构体而不是 `info reg` 的文本：`analyze` / `forensic` 现在靠正则从
/// 那段表格里捞 `pc`/`sp`/`lr`，而正则对版式敏感 —— 0.32 的 `r15` 写法会让
/// `forensic` 取不到 pc（`parse_regs` 只认 `pc`，没有 `r15` 回退）。
/// 结构体字段没有这个问题，也顺手把"同一时刻"这件事做实了。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Registers {
    pub pc: u64,
    pub sp: u64,
    pub lr: u64,
    pub xpsr: u64,
    pub msp: u64,
    pub psp: u64,
    /// r0..=r12
    pub r: [u64; 13],
    /// True when this snapshot had to halt a running core. The target is left halted;
    /// callers that show the snapshot must say so instead of implying a passive read.
    pub halted_for_read: bool,
}

impl Registers {
    /// 按名字取一个通用寄存器（r0..r12），越界返回 None。
    pub fn gpr(&self, n: usize) -> Option<u64> {
        self.r.get(n).copied()
    }
}

/// 读一次完整的寄存器组。**运行中的核会被停住并保持停住**：Cortex-M 的核寄存器
/// 经 DCRSR/DCRDR 读，目标在跑时读数不可靠，CLI 路径同样先 `-c break` 再
/// `info reg`。`halted_for_read` 记录这次读是否改变了运行状态，调用方必须把它
/// 告诉用户，不能把一次有副作用的读说成“不改变运行状态”。
///
/// 单个寄存器读失败记 0：MSP/PSP 在只有单栈的核上可能不存在，一个可选寄存器
/// 缺失不该让整张表失败。
pub fn registers(chip: &str, probe: Option<&str>) -> Result<Registers, ProbeError> {
    let _lease = ProbeLease::acquire("a library register read").map_err(ProbeError::Failed)?;
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        // 先停核；已经停下时 `halt` 是幂等的。
        let was_running = !matches!(core.status(), Ok(probe_rs::CoreStatus::Halted(_)));
        let _ = core.halt(Duration::from_millis(500));
        let mut regs = snapshot(&mut core)?;
        regs.halted_for_read = was_running;
        Ok(regs)
    })
}

/// 读一次寄存器快照。调用方负责让核处于可读状态（停下）。
fn snapshot(core: &mut probe_rs::Core<'_>) -> Result<Registers, ProbeError> {
    // `registers()` 返回 `&'static CoreRegisters`，不借用 `core`，
    // 所以下面可以一边遍历描述、一边 `read_core_reg`。
    let described = core.registers();
    let mut out = Registers::default();
    let mut read_ok = 0usize;
    let mut first_err: Option<String> = None;
    for r in described.core_registers() {
        let Ok(v) = core.read_core_reg::<u32>(r.id()) else {
            if first_err.is_none() {
                first_err = Some(format!("register {} could not be read", r.name()));
            }
            continue;
        };
        read_ok += 1;
        let v = u64::from(v);
        let name = r.name().to_ascii_lowercase();
        match name.as_str() {
            "pc" | "r15" => out.pc = v,
            "sp" | "r13" => out.sp = v,
            "lr" | "r14" => out.lr = v,
            "xpsr" => out.xpsr = v,
            "msp" => out.msp = v,
            "psp" => out.psp = v,
            other => {
                if let Some(n) = other
                    .strip_prefix('r')
                    .and_then(|d| d.parse::<usize>().ok())
                    .filter(|n| *n < 13)
                {
                    out.r[n] = v;
                }
            }
        }
    }
    // 一个都读不到就是失败，不能把"全 0"当成快照交出去 ——
    // 那会让 analyze 报"pc=0"，看起来像固件跳飞了。
    if read_ok == 0 {
        return Err(ProbeError::Failed(format!(
            "[Io] no core register could be read{}",
            first_err.map(|e| format!(" ({e})")).unwrap_or_default()
        )));
    }
    Ok(out)
}

/// 这台机器上现在能看到几个探针（只枚举，不 attach）。
///
/// `doctor` 用它回答"能不能烧/读"，而不要求 CLI 在 PATH 上 —— 库已经链进来了，
/// 能力不该由"有没有装那个命令行"决定。
pub fn probes_visible() -> usize {
    probe_rs::probe::list::Lister::new().list_all().len()
}

/// 从一个 ELF 里取出要写进 flash 的段。
///
/// probe-rs 0.32 不再提供 ELF 读取入口（`builtin-formats` 不在它的 feature 列表里），
/// 所以段由我们自己解析 —— 仓库里本来就有 `object`，`elf_analyze` 一直在用。
/// 只取 `PT_LOAD` 且非空的段：地址 0 与空段不是要烧的内容。
fn elf_segments(bytes: &[u8]) -> Result<Vec<(u64, Vec<u8>)>, ProbeError> {
    use object::read::elf::{ElfFile32, ElfFile64};

    if bytes.len() < 5 {
        return Err(ProbeError::Failed(
            "[InvalidInput] not a readable ELF: file is too short".to_string(),
        ));
    }
    // `ObjectSegment::address()` is p_vaddr. A `.data : { } > RAM AT > FLASH`
    // segment has p_vaddr in RAM and p_paddr in FLASH; programming the vaddr
    // puts the initial values in RAM (where the startup code then overwrites
    // them from erased FLASH) and reports success. p_paddr is the load address.
    match bytes[4] {
        1 => segments_in(
            &ElfFile32::<object::Endianness>::parse(bytes).map_err(|e| {
                ProbeError::Failed(format!("[InvalidInput] not a readable ELF: {e}"))
            })?,
            bytes,
        ),
        2 => segments_in(
            &ElfFile64::<object::Endianness>::parse(bytes).map_err(|e| {
                ProbeError::Failed(format!("[InvalidInput] not a readable ELF: {e}"))
            })?,
            bytes,
        ),
        other => Err(ProbeError::Failed(format!(
            "[InvalidInput] unsupported ELF class {other}"
        ))),
    }
}

fn segments_in<'data, Elf>(
    file: &object::read::elf::ElfFile<'data, Elf, &'data [u8]>,
    bytes: &'data [u8],
) -> Result<Vec<(u64, Vec<u8>)>, ProbeError>
where
    Elf: object::read::elf::FileHeader,
{
    use object::read::elf::ProgramHeader as _;

    let endian = file
        .elf_header()
        .endian()
        .map_err(|e| ProbeError::Failed(format!("[InvalidInput] ELF endianness: {e}")))?;
    let mut out = Vec::new();
    for seg in file.elf_program_headers() {
        if seg.p_type(endian) != object::elf::PT_LOAD {
            continue;
        }
        let addr: u64 = seg.p_paddr(endian).into();
        if addr == 0 {
            continue;
        }
        let data = seg.data(endian, bytes).map_err(|_| {
            ProbeError::Failed("[InvalidInput] unreadable ELF segment (bad file range)".to_string())
        })?;
        if data.is_empty() {
            continue;
        }
        out.push((addr, data.to_vec()));
    }
    if out.is_empty() {
        return Err(ProbeError::Failed(
            "[InvalidInput] the ELF has no loadable segments to flash".to_string(),
        ));
    }
    Ok(out)
}

/// 把 ELF 烧进目标，返回写入的段数。
///
/// 会话与读路径共用：一次 `flash` 之后紧接着 `debug mem` 不必再 attach 一次。
/// `commit` 内部完成擦除与写入；`reset` 由调用方决定，`false` 时目标保持
/// 烧录算法留下的状态，调用方不得打印“target reset”。
pub fn flash_elf(
    chip: &str,
    probe: Option<&str>,
    elf: &std::path::Path,
) -> Result<usize, ProbeError> {
    flash_elf_with_reset(chip, probe, elf, true)
}

pub fn flash_elf_with_reset(
    chip: &str,
    probe: Option<&str>,
    elf: &std::path::Path,
    reset: bool,
) -> Result<usize, ProbeError> {
    use probe_rs::config::TargetDescriptionSource;
    use probe_rs::flashing::{DownloadOptions, FlashLoader};

    // The lease is the busy marker for a long library flash: if this call is
    // abandoned on timeout, the blocking task keeps the lease until probe-rs
    // returns, and every later CLI/library probe operation is refused instead
    // of talking to a target that is still being erased.
    let _lease = ProbeLease::acquire("a library flash").map_err(ProbeError::Failed)?;
    let bytes = std::fs::read(elf)
        .map_err(|e| ProbeError::Failed(format!("[Io] cannot read {}: {e}", elf.display())))?;
    let segments = elf_segments(&bytes)?;
    let n = segments.len();

    with_session(chip, probe, |session| {
        let memory_map = session.target().memory_map.clone();
        let mut loader = FlashLoader::new(memory_map, TargetDescriptionSource::BuiltIn);
        for (addr, data) in &segments {
            loader.add_data(*addr, data).map_err(|e| {
                ProbeError::Failed(format!(
                    "[Io] flash loader refused segment at {addr:#010x} ({} bytes): {e}",
                    data.len()
                ))
            })?;
        }
        loader
            .commit(session, DownloadOptions::default())
            .map_err(|e| ProbeError::Failed(format!("[Io] flash commit failed: {e}")))?;
        if reset {
            let mut core = core(session)?;
            core.reset()
                .map_err(|e| ProbeError::Failed(format!("[Io] reset after flash: {e}")))?;
        }
        Ok(())
    })?;
    Ok(n)
}

/// 释放缓存会话。**调用点只有两个尾站点**：`tools::util::run_argv` 与
/// `run_streaming` 在拿到探针租约之后、spawn `probe-rs` 之前。库里的
/// `Session` 持有 USB 句柄，`probe-rs attach` / `probe-rs run` 会以
/// "failed to open device" 失败 —— 2026-10-19 在 `run` 的回退路径上实测到了
/// 这个错误。把释放放在单个 spawn 尾站点，是为了让 `debug`/`hil` 的未来回退
/// 也自动继承这条规则；直接在这里调用会绕过租约，并可能在后台烧录仍持锁时
/// 卡在 session 互斥量上。
pub fn release() {
    if let Ok(mut slot) = SESSION.lock() {
        *slot = None;
    }
}

// 进程退出时不做显式释放：闲置 30 秒（`IDLE_RELEASE`）会自己放掉探针，
// 而在 `main` 的每条返回路径上插一句 release 只会多出几个漏掉的出口。

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF32 with one PT_LOAD whose p_vaddr and p_paddr differ, which is the
    /// normal `.data : { } > RAM AT > FLASH` layout. Programming `address()` (p_vaddr)
    /// would put the initial values in RAM and leave FLASH erased; the loader must use
    /// p_paddr.
    fn synthetic_elf32_with_split_load_address() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x104];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1; // ELFCLASS32
        bytes[5] = 1; // little endian
        bytes[6] = 1; // EV_CURRENT
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        bytes[18..20].copy_from_slice(&40u16.to_le_bytes()); // EM_ARM
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
        bytes[24..28].copy_from_slice(&0x0800_0000u32.to_le_bytes()); // entry
        bytes[28..32].copy_from_slice(&52u32.to_le_bytes()); // e_phoff
        bytes[40..42].copy_from_slice(&52u16.to_le_bytes()); // e_ehsize
        bytes[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
        bytes[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let ph = 52;
        bytes[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        bytes[ph + 4..ph + 8].copy_from_slice(&0x100u32.to_le_bytes()); // p_offset
        bytes[ph + 8..ph + 12].copy_from_slice(&0x2000_0000u32.to_le_bytes()); // p_vaddr
        bytes[ph + 12..ph + 16].copy_from_slice(&0x0800_0000u32.to_le_bytes()); // p_paddr
        bytes[ph + 16..ph + 20].copy_from_slice(&4u32.to_le_bytes()); // p_filesz
        bytes[ph + 20..ph + 24].copy_from_slice(&4u32.to_le_bytes()); // p_memsz
        bytes[ph + 24..ph + 28].copy_from_slice(&5u32.to_le_bytes()); // p_flags
        bytes[ph + 28..ph + 32].copy_from_slice(&4u32.to_le_bytes()); // p_align
        bytes[0x100..0x104].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        bytes
    }

    #[test]
    fn elf_segments_uses_the_load_address_not_the_virtual_address() {
        let bytes = synthetic_elf32_with_split_load_address();
        let segments = elf_segments(&bytes).expect("the synthetic ELF must parse");
        assert_eq!(segments.len(), 1, "one PT_LOAD: {segments:?}");
        assert_eq!(
            segments[0].0, 0x0800_0000,
            "must program p_paddr; p_vaddr 0x2000_0000 is RAM and the reset would erase it"
        );
        assert_eq!(segments[0].1, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    /// 这台机器上没有探针时（CI 也一样），`open` 必须是 `Unavailable` 而不是 panic
    /// —— 它决定调用方要不要回退 CLI，判错的代价是一个本来能用的功能直接报错。
    /// 需要真板子（NUCLEO-G431RB 已连接）：
    ///
    /// ```text
    /// cargo test -p firment-tools --lib reads_gpioc_through_the_library -- --ignored
    /// ```
    ///
    /// 这条测试存在的理由：`debug mem` 现在优先走库，而库能不能 attach、`read_32`
    /// 的地址语义对不对，只有真探针能回答。CI 没有板子，所以它 `#[ignore]`，
    /// 但它是"集成"这两个字唯一的证据。
    #[test]
    #[ignore = "needs a NUCLEO-G431RB attached"]
    fn reads_gpioc_through_the_library() {
        let words = read_words("stm32g431rb", None, 0x4800_0800, 6).expect("attach + read");
        assert_eq!(words.len(), 6, "six GPIOC registers");
        // 全 0xFFFFFFFF 不是一块活着的 MCU 会给出的快照。
        assert!(
            words.iter().any(|w| *w != u32::MAX),
            "GPIOC read back as all-ones: {words:x?}"
        );
    }

    /// 需要真板子 + 一份固件：
    ///
    /// ```text
    /// FIRMENT_TEST_ELF=/path/to/firmware.elf \
    ///   cargo test -p firment-tools --lib flashes_a_real_elf -- --ignored
    /// ```
    ///
    /// 它证明的是"库能擦、能写、能复位"，而不是"CLI 能"——后者已经在 24 局基准里跑过。
    #[test]
    #[ignore = "needs a NUCLEO-G431RB and a firmware ELF"]
    fn flashes_a_real_elf_through_the_library() {
        let path =
            std::env::var("FIRMENT_TEST_ELF").expect("set FIRMENT_TEST_ELF to a firmware .elf");
        let n = flash_elf("stm32g431rb", None, std::path::Path::new(&path)).expect("flash");
        assert!(n >= 1, "expected at least one loadable segment, wrote {n}");
    }

    #[test]
    fn attaching_without_a_probe_is_unavailable_not_failed() {
        // 用一个不可能存在的芯片名，把失败挡在"找芯片"这一步，不依赖有没有插探针。
        match open("not-a-real-chip-xyz", None) {
            Err(ProbeError::Unavailable(m)) => assert!(m.contains("NoProbe"), "{m}"),
            Err(ProbeError::Failed(m)) => panic!("expected Unavailable, got Failed: {m}"),
            Ok(_) => panic!("attached to a chip that does not exist"),
        }
    }
}
