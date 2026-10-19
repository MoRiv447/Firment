//! sigrok 能力清单：`sigrok-cli -L` 的类型化结果，进程内缓存一次。
//!
//! 为什么要有这一层：`la` 的 schema 里写着 `decoder` 可以是 `uart`/`spi`/`i2c`…，
//! 但**装没装那个 decoder 是这台机器的事实**，不是 schema 能保证的。没有这份清单时，
//! 模型会挑一个本机没有的 decoder，跑到一半拿到一句 sigrok 原文。清单把这件事前移：
//! 能力在进程里查一次，`decode` 在启动子进程之前就能拒绝，并给出这个 build 里真实存在
//! 的相近名字。
//!
//! 它同时是"适配层"的第一块：外部程序的**能力**成了我们自己的数据，
//! 而不是每次从 stderr 里猜。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::la_cmd::parse_sigrok_version;

/// 一次 `-L` 得到的、我们真正会用的那部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// `sigrok-cli` 的版本三元组，和 `la_cmd::parse_sigrok_version` 同一形状。
    pub version: Option<(u8, u8, u8)>,
    /// 硬件驱动名，小写。
    pub drivers: Vec<String>,
    /// 协议 decoder 名，小写。
    pub decoders: Vec<String>,
}

impl Capabilities {
    /// 解析 `sigrok-cli -L` 的输出。
    ///
    /// 输出分五节（drivers / input formats / output formats / transform modules /
    /// decoders），每节形如：
    ///
    /// ```text
    /// Supported protocol decoders:
    ///   ac97                 Audio Codec 97
    ///   uart                 Universal Asynchronous Receiver/Transmitter
    /// ```
    ///
    /// 每行取第一个 token 作为名字；`Supported …:` 那一行和空行跳过。只收名字里
    /// 只有 `[a-z0-9_-]` 的 token —— 描述里的词不会混进名单。
    pub fn parse(list_text: &str, version_text: &str) -> Self {
        let mut drivers = Vec::new();
        let mut decoders = Vec::new();
        let mut section = "";
        for line in list_text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("Supported ") {
                section = rest.trim_end_matches(':').trim();
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            let Some(name) = trimmed.split_whitespace().next() else {
                continue;
            };
            if !is_capability_name(name) {
                continue;
            }
            let name = name.to_ascii_lowercase();
            match section {
                "hardware drivers" => drivers.push(name),
                "protocol decoders" => decoders.push(name),
                _ => {}
            }
        }
        Self {
            version: parse_sigrok_version(version_text),
            drivers,
            decoders,
        }
    }

    /// 名字比较：小写，`-` 与 `_` 视为同一个字符（sigrok 两个都见过）。
    fn same(a: &str, b: &str) -> bool {
        a.to_ascii_lowercase().replace('_', "-") == b.to_ascii_lowercase().replace('_', "-")
    }

    pub fn has_driver(&self, name: &str) -> bool {
        self.drivers.iter().any(|d| Self::same(d, name))
    }

    pub fn has_decoder(&self, name: &str) -> bool {
        self.decoders.iter().any(|d| Self::same(d, name))
    }

    /// 名字打错时给出这个 build 里真实存在的相近项（最多 5 个）。
    ///
    /// 只做"包含"匹配，不做编辑距离：对一个几十项的表，子串已经足够把
    /// `uart` → `uart`、`i2c` → `i2c-filter` 这类关系找出来，而编辑距离会把
    /// 不相干的名字也拉进来。
    pub fn similar_decoders(&self, want: &str) -> Vec<String> {
        let want = want.to_ascii_lowercase();
        let mut out: Vec<String> = self
            .decoders
            .iter()
            .filter(|d| d.contains(&want) || want.contains(d.as_str()))
            .take(5)
            .cloned()
            .collect();
        if out.is_empty() {
            out = self.decoders.iter().take(5).cloned().collect();
        }
        out
    }
}

/// sigrok 的名字都是小写 + 数字 + `-`/`_`；描述里的词（含大写、标点）会被挡掉。
fn is_capability_name(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// 进程内缓存：一台机器上 sigrok 的 build 在一次运行里不会变。
static CACHE: OnceLock<Mutex<HashMap<String, Arc<Capabilities>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Arc<Capabilities>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取缓存的清单；没有就是 `None`。
pub(crate) fn cached(bin: &str) -> Option<Arc<Capabilities>> {
    cache().lock().ok().and_then(|m| m.get(bin).cloned())
}

/// 记下清单（同一 bin 后写覆盖前写）。
pub(crate) fn remember(bin: &str, caps: Capabilities) {
    if let Ok(mut m) = cache().lock() {
        m.insert(bin.to_string(), Arc::new(caps));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
The following devices were found:
sigrok-cli 0.7.2

Supported hardware drivers:
  fx2lafw              Cypress FX2LP based device
  saleae-logic16       Saleae Logic16
  demo                 Demo driver

Supported input formats:
  binary               Binary input
  vcd                  Value Change Dump

Supported output formats:
  srzip                Sigma/II
  bits                 Bitstream

Supported transform modules:
  scale                Scale

Supported protocol decoders:
  ac97                 Audio Codec 97
  i2c                  Inter-Integrated Circuit
  uart                 Universal Asynchronous Receiver/Transmitter
  uart_filter          UART filter
";

    fn caps() -> Capabilities {
        Capabilities::parse(LIST, "sigrok-cli 0.7.2")
    }

    #[test]
    fn each_section_is_read_and_formats_do_not_leak_into_drivers() {
        let c = caps();
        assert_eq!(c.drivers, vec!["fx2lafw", "saleae-logic16", "demo"]);
        // 保留 sigrok 自己的拼写：这张表是用来**报告**的，归一化只发生在比较时。
        assert_eq!(c.decoders, vec!["ac97", "i2c", "uart", "uart_filter"]);
        // 这一条是旧 `drivers_section` 注释里点名的坑：`srzip` 是 output format，
        // 不是驱动；`uart` 是 decoder，也不是驱动。
        assert!(!c.has_driver("srzip"));
        assert!(!c.has_driver("uart"));
        assert!(c.has_driver("fx2lafw"));
    }

    #[test]
    fn version_is_parsed_from_the_banner() {
        assert_eq!(caps().version, Some((0, 7, 2)));
        assert_eq!(Capabilities::parse(LIST, "no banner").version, None);
    }

    #[test]
    fn underscore_and_dash_are_the_same_name() {
        let c = caps();
        assert!(c.has_decoder("uart_filter"));
        assert!(c.has_decoder("uart-filter"));
        assert!(c.has_decoder("UART"));
    }

    #[test]
    fn a_missing_decoder_gets_real_names_back() {
        let c = caps();
        assert!(!c.has_decoder("swd"));
        let near = c.similar_decoders("uart");
        assert!(near.contains(&"uart".to_string()), "{near:?}");
        // 完全对不上时给出表里的前几个，而不是空列表——模型至少知道这个 build 有什么。
        assert!(!c.similar_decoders("zzz").is_empty());
    }

    #[test]
    fn an_empty_or_garbage_list_is_not_a_crash() {
        let c = Capabilities::parse("", "");
        assert!(c.drivers.is_empty() && c.decoders.is_empty());
        assert!(!c.has_decoder("uart"));
    }
}
