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
pub(crate) enum ProbeError {
    /// 库拿不到探针/芯片：退回 CLI 是合理的（CLI 可能配了别的探针选择方式）。
    Unavailable(String),
    /// 拿到了，操作失败：这是真错误，回退只会把同一个错误再报一次。
    Failed(String),
}

impl ProbeError {
    pub(crate) fn message(&self) -> &str {
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
pub(crate) fn read_words(
    chip: &str,
    probe: Option<&str>,
    addr: u64,
    n: usize,
) -> Result<Vec<u32>, ProbeError> {
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        let mut words = vec![0u32; n];
        core.read_32(addr, &mut words)
            .map_err(|e| ProbeError::Failed(format!("[Io] read {n} words at {addr:#010x}: {e}")))?;
        Ok(words)
    })
}

/// 写一个 32 位字。**调用方负责知道自己在写什么**（见 `debug` 的 GPIO 警告）。
pub(crate) fn write_word(
    chip: &str,
    probe: Option<&str>,
    addr: u64,
    value: u32,
) -> Result<(), ProbeError> {
    with_session(chip, probe, |session| {
        let mut core = core(session)?;
        core.write_word_32(addr, value)
            .map_err(|e| ProbeError::Failed(format!("[Io] write {value:#x} to {addr:#010x}: {e}")))
    })
}

// halt / resume 先不包：它们要和 flash/run 一起切过来才有意义，
// 单独切换会让"CLI 烧录 + 库 halt"这种混搭状态更难解释。

/// 从一个 ELF 里取出要写进 flash 的段。
///
/// probe-rs 0.32 不再提供 ELF 读取入口（`builtin-formats` 不在它的 feature 列表里），
/// 所以段由我们自己解析 —— 仓库里本来就有 `object`，`elf_analyze` 一直在用。
/// 只取 `PT_LOAD` 且非空的段：地址 0 与空段不是要烧的内容。
fn elf_segments(bytes: &[u8]) -> Result<Vec<(u64, Vec<u8>)>, ProbeError> {
    use object::{Object as _, ObjectSegment as _};
    let file = object::File::parse(bytes)
        .map_err(|e| ProbeError::Failed(format!("[InvalidInput] not a readable ELF: {e}")))?;
    let mut out = Vec::new();
    for seg in file.segments() {
        let addr = seg.address();
        if addr == 0 {
            continue;
        }
        let data = seg.data().map_err(|e| {
            ProbeError::Failed(format!("[InvalidInput] unreadable ELF segment: {e}"))
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
/// `commit` 内部完成擦除与写入，`reset` 让目标从新固件启动。
pub(crate) fn flash_elf(
    chip: &str,
    probe: Option<&str>,
    elf: &std::path::Path,
) -> Result<usize, ProbeError> {
    use probe_rs::config::TargetDescriptionSource;
    use probe_rs::flashing::{DownloadOptions, FlashLoader};

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
        let mut core = core(session)?;
        core.reset()
            .map_err(|e| ProbeError::Failed(format!("[Io] reset after flash: {e}")))?;
        Ok(())
    })?;
    Ok(n)
}

// 进程退出时不做显式释放：闲置 30 秒（`IDLE_RELEASE`）会自己放掉探针，
// 而在 `main` 的每条返回路径上插一句 release 只会多出几个漏掉的出口。

#[cfg(test)]
mod tests {
    use super::*;

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
