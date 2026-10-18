//! GPIO 寄存器字段解码（只解码，不判断）。
//!
//! 为什么需要它：2026-10-18 基准 case04 的一局里，agent 用
//! `debug mem 0x48000800` 读了 GPIOC 的 6 个字，拿到
//! `f3ffffff 00000000 00000000 04000000 00002000 00000000`，
//! 然后把 `IDR bit13 = 1` 解读成"外部上拉，按键低有效"——
//! 而同一份快照里 `PUPDR = 0x04000000` 就是 **PC13 内部上拉**：
//! 那个 1 是它自己的配置造出来的，不是外部电路。
//!
//! 原始字不解释字段，等于把"读寄存器"变成"猜寄存器"。这里把每个字拆成
//! `MODER/PUPDR/IDR/ODR`（或 F1 的 CRL/CRH）的可读字段；至于"这意味着什么"，
//! 仍然由调用者判断。
//!
//! **布局不写死在代码里**：每个家族的基址与寄存器编码放在知识库的
//! `cheatsheets/gpio-layouts.toml`（和数据手册同一处出处），代码只解析。
//! 只写被测那一块板会变成"为测试优化"；写全 KB 里已有的家族才是产品行为。
//! 不认识的芯片**明说没有布局**，而不是沉默 —— 沉默正是本文件要消除的毛病。

use std::sync::OnceLock;

/// 一个芯片家族的 GPIO 布局。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// 芯片名前缀，小写，例如 `stm32g4`（匹配 `stm32g431rb`）。
    pub chip_prefix: String,
    pub kind: Kind,
    /// 端口 A 的基址，端口之间间隔 `stride`。
    pub base: u64,
    pub stride: u64,
    /// 出处（手册章节），必填：数值写错比没有更糟。
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    /// MODER/PUPDR/IDR/ODR（F0/F3/F4/G0/G4/L4/U5/H7…）
    Moder,
    /// CRL/CRH 的 4 位 CNF+MODE（F1 一类）
    CrlCrh,
}

const PORT_NAMES: [&str; 8] = ["A", "B", "C", "D", "E", "F", "G", "H"];

/// 解析 `cheatsheets/gpio-layouts.toml`。
///
/// 数值校验：基址按 stride 对齐、stride 不小于一个端口的间隔、出处非空。
/// 一个错位的 base 会让解码输出"权威的错误值"，比不输出更糟，所以坏条目**丢弃**
/// 而不是带着用。
fn parse_layouts(text: &str) -> Vec<Layout> {
    let Ok(doc) = toml::from_str::<toml::Value>(text) else {
        return Vec::new();
    };
    let Some(rows) = doc.get("layout").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows {
        let kind = match row.get("kind").and_then(|v| v.as_str()) {
            Some("moder") => Kind::Moder,
            Some("crl_crh") => Kind::CrlCrh,
            _ => continue,
        };
        let Some(base) = row.get("base").and_then(|v| v.as_integer()) else {
            continue;
        };
        let Some(stride) = row.get("stride").and_then(|v| v.as_integer()) else {
            continue;
        };
        let source = row
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let chip_prefix = row
            .get("chip_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if base <= 0 || stride < 0x400 || !(base as u64).is_multiple_of(stride as u64) {
            continue;
        }
        if chip_prefix.is_empty() || source.is_empty() {
            continue;
        }
        out.push(Layout {
            chip_prefix,
            kind,
            base: base as u64,
            stride: stride as u64,
            source,
        });
    }
    out
}

/// 从知识库读布局表（进程内只读一次）。
///
/// 路径来自 `firment_core::kb::seed_kb_dir()`，和 cheatsheet 同一棵树 ——
/// 数据手册的出处和代码用的数值在同一个文件里，改一个不会忘了另一个。
fn layouts() -> &'static [Layout] {
    static CACHE: OnceLock<Vec<Layout>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let path = firment_core::kb::seed_kb_dir().join("cheatsheets/gpio-layouts.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => parse_layouts(&text),
            Err(_) => Vec::new(),
        }
    })
}

/// 按芯片名找布局。`chip` 为 `None` 时无法匹配 —— 调用方应据此明说"不知道"。
pub(crate) fn layout_for(chip: Option<&str>) -> Option<&'static Layout> {
    let name = chip?.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    layouts().iter().find(|l| name.starts_with(&l.chip_prefix))
}

/// 端口索引 → (名字, 基址)。
fn port_at(layout: &Layout, addr: u64) -> Option<(&'static str, u64)> {
    let off = addr.checked_sub(layout.base)?;
    let idx = (off / layout.stride) as usize;
    if idx >= PORT_NAMES.len() {
        return None;
    }
    Some((PORT_NAMES[idx], layout.base + (idx as u64) * layout.stride))
}

/// 这个地址是不是某个已知端口的 GPIO 区间（写 GPIO 时给警告用）。
pub(crate) fn is_gpio(chip: Option<&str>, addr: u64) -> bool {
    layout_for(chip).and_then(|l| port_at(l, addr)).is_some()
}

/// 从 probe-rs 的读内存输出里取出 32 位字。
///
/// 输出形状随版本略有差别（`0x48000800 = f3ffffff` 或裸的
/// `f3ffffff 00000000 …`），所以只认"恰好 8 个十六进制字符"的 token，
/// 最多取 `limit` 个 —— 不猜、不补零。带 `0x` 前缀的是**地址**，跳过。
pub(crate) fn parse_words(text: &str, limit: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| c.is_whitespace() || c == ',') {
        if tok.starts_with("0x") || tok.starts_with("0X") {
            continue;
        }
        if tok.len() == 8
            && tok.chars().all(|c| c.is_ascii_hexdigit())
            && let Ok(v) = u32::from_str_radix(tok, 16)
        {
            out.push(v);
            if out.len() == limit {
                break;
            }
        }
    }
    out
}

fn mode_name(v: u32) -> &'static str {
    match v & 0b11 {
        0b00 => "input",
        0b01 => "output",
        0b10 => "alternate",
        _ => "analog",
    }
}

fn pull_name(v: u32) -> &'static str {
    match v & 0b11 {
        0b00 => "none",
        0b01 => "pull-up",
        0b10 => "pull-down",
        _ => "reserved",
    }
}

fn push_pairs(
    parts: &mut Vec<String>,
    reg: &str,
    port: &str,
    w: u32,
    f: &dyn Fn(u32) -> Option<&'static str>,
) {
    let s = (0..16)
        .filter_map(|p| f((w >> (2 * p)) & 0b11).map(|n| format!("P{port}{p}={n}")))
        .collect::<Vec<_>>()
        .join(",");
    if !s.is_empty() {
        parts.push(format!("{reg} {s}"));
    }
}

fn push_high_pins(parts: &mut Vec<String>, reg: &str, port: &str, w: u32) {
    let s = (0..16)
        .filter(|p| w & (1 << p) != 0)
        .map(|p| format!("P{port}{p}"))
        .collect::<Vec<_>>()
        .join(",");
    if !s.is_empty() {
        parts.push(format!("{reg} {s}=1"));
    }
}

/// 按布局解码一组 32 位字，返回一行可读文本。
///
/// 只列**非默认/非零**的位：`MODER` 只列不是 input 的脚，`PUPDR` 只列有上下拉的脚，
/// `IDR/ODR` 只列读作 1 的脚。全 0 的端口不该刷屏，该一眼看出"哪个脚被动过"。
fn decode_with(layout: &Layout, addr: u64, words: &[u32]) -> Option<String> {
    let (port, base) = port_at(layout, addr)?;
    let mut parts: Vec<String> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let Some(reg) = (addr + 4 * i as u64).checked_sub(base) else {
            break;
        };
        match layout.kind {
            Kind::Moder => match reg {
                0x00 => push_pairs(&mut parts, "MODER", port, *w, &|v| match v & 0b11 {
                    0 => None,
                    _ => Some(mode_name(v)),
                }),
                0x0C => push_pairs(&mut parts, "PUPDR", port, *w, &|v| match v & 0b11 {
                    0 => None,
                    _ => Some(pull_name(v)),
                }),
                0x10 | 0x14 => {
                    let name = if reg == 0x10 { "IDR" } else { "ODR" };
                    push_high_pins(&mut parts, name, port, *w);
                }
                _ => {}
            },
            // CRL 管 0..7，CRH 管 8..15；每脚 4 位：CNF[1:0] + MODE[1:0]。
            Kind::CrlCrh => match reg {
                0x00 | 0x04 => {
                    let first = if reg == 0x00 { 0 } else { 8 };
                    for p in 0..8 {
                        let nib = (w >> (4 * p)) & 0xF;
                        let cnf = (nib >> 2) & 0b11;
                        let what = match (nib & 0b11, cnf) {
                            (0, 0) => None, // 复位值：模拟输入，不刷屏
                            (0, 1) => Some("input floating"),
                            (0, _) => Some("input pull"),
                            (_, 0) => Some("output push-pull"),
                            (_, 1) => Some("output open-drain"),
                            (_, 2) => Some("alternate push-pull"),
                            _ => Some("alternate open-drain"),
                        };
                        if let Some(name) = what {
                            parts.push(format!("P{port}{}={name}", first + p));
                        }
                    }
                }
                0x08 | 0x0C => {
                    let name = if reg == 0x08 { "IDR" } else { "ODR" };
                    push_high_pins(&mut parts, name, port, *w);
                }
                _ => {}
            },
        }
    }
    if parts.is_empty() {
        return None;
    }
    let kind = match layout.kind {
        Kind::Moder => "moder",
        Kind::CrlCrh => "crl_crh",
    };
    Some(format!("GPIO{port} decode [{kind}]: {}", parts.join("; ")))
}

/// 按芯片名解码；找不到布局时返回 `None`（由调用方决定怎么提示）。
pub(crate) fn decode(chip: Option<&str>, addr: u64, words: &[u32]) -> Option<String> {
    decode_with(layout_for(chip)?, addr, words)
}

/// 芯片没有布局时，给一句"我不知道"，而不是让原始字孤零零地待着。
///
/// 只在"这看起来像外设访问"时提示（0x4000_0000..0x6000_0000）：读 RAM/Flash
/// 时不该刷这句。
pub(crate) fn unknown_layout_note(chip: Option<&str>, addr: u64) -> Option<String> {
    if layout_for(chip).is_some() || !(0x4000_0000..0x6000_0000).contains(&addr) {
        return None;
    }
    let name = chip.unwrap_or("<unset>");
    Some(format!(
        "note: no GPIO layout known for chip '{name}' — raw words only. Add a [[layout]] \
         entry to cheatsheets/gpio-layouts.toml (base + source) to get field decoding."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g4() -> Layout {
        Layout {
            chip_prefix: "stm32g4".into(),
            kind: Kind::Moder,
            base: 0x4800_0000,
            stride: 0x400,
            source: "RM0440 §9.4".into(),
        }
    }

    /// 2026-10-18 的真实快照：agent 把 `IDR=1` 当成外部上拉，而 PUPDR 就是上拉。
    #[test]
    fn internal_pull_up_is_named_so_a_high_read_is_not_blamed_on_wiring() {
        let words = [
            0xf3ff_ffff, // MODER
            0x0000_0000, // OTYPER
            0x0000_0000, // OSPEEDR
            0x0400_0000, // PUPDR: PC13 = 0b01 = pull-up
            0x0000_2000, // IDR:   PC13 = 1
            0x0000_0000, // ODR
        ];
        let d = decode_with(&g4(), 0x4800_0800, &words).expect("GPIOC 应当能解码");
        assert!(d.contains("PUPDR PC13=pull-up"), "{d}");
        assert!(d.contains("IDR PC13=1"), "{d}");
    }

    #[test]
    fn pull_down_and_output_modes_are_named() {
        let words = [0x0000_0400, 0, 0, 0x0000_0800, 0, 0x0000_0020];
        let d = decode_with(&g4(), 0x4800_0000, &words).unwrap();
        assert!(d.contains("MODER PA5=output"), "{d}");
        assert!(d.contains("PUPDR PA5=pull-down"), "{d}");
        assert!(d.contains("ODR PA5=1"), "{d}");
    }

    #[test]
    fn a_single_register_read_still_decodes() {
        let d = decode_with(&g4(), 0x4800_0810, &[0x0000_2000]).unwrap();
        assert!(d.contains("IDR PC13=1"), "{d}");
    }

    #[test]
    fn non_gpio_addresses_and_all_zero_ports_stay_quiet() {
        assert!(decode_with(&g4(), 0x2000_0000, &[1, 2, 3]).is_none());
        assert!(decode_with(&g4(), 0x4800_0800, &[0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn words_are_parsed_from_either_output_shape() {
        assert_eq!(parse_words("0x48000800 = f3ffffff\n", 4), vec![0xf3ff_ffff]);
        assert_eq!(
            parse_words("f3ffffff 00000000 04000000", 2),
            vec![0xf3ff_ffff, 0]
        );
    }

    /// F1 是另一套编码：CRL 的 4 位 CNF+MODE。没有这条，表里写 `crl_crh` 的家族
    /// 会被当成 moder 解出胡说八道。
    #[test]
    fn f1_crl_crh_encoding_is_decoded_separately() {
        let f1 = Layout {
            chip_prefix: "stm32f1".into(),
            kind: Kind::CrlCrh,
            base: 0x4001_0800,
            stride: 0x400,
            source: "RM0008 §9.2".into(),
        };
        // PA5: CNF=00 MODE=11 -> output push-pull；PA0: CNF=10 MODE=00 -> input pull
        let crl = (0b0011u32 << 20) | 0b1000;
        let d = decode_with(&f1, 0x4001_0800, &[crl, 0, 0x0000_0020]).unwrap();
        assert!(d.contains("PA5=output push-pull"), "{d}");
        assert!(d.contains("PA0=input pull"), "{d}");
        assert!(d.contains("IDR PA5=1"), "{d}");
    }

    #[test]
    fn layouts_are_parsed_and_bad_ones_dropped() {
        let text = r#"
[[layout]]
chip_prefix = "stm32g4"
kind = "moder"
base = 0x48000000
stride = 0x400
source = "RM0440"

[[layout]]
chip_prefix = "broken"
kind = "moder"
base = 0x48000001
stride = 0x400
source = ""
"#;
        let ls = parse_layouts(text);
        assert_eq!(ls.len(), 1, "缺出处/未对齐的条目要丢掉：{ls:?}");
        assert_eq!(ls[0].chip_prefix, "stm32g4");
    }

    #[test]
    fn chip_matching_uses_the_prefix_and_reports_unknown_explicitly() {
        let ls = layouts();
        if ls.is_empty() {
            return; // KB 没 seed 到（CI 里可能如此）—— 这条测试不据此判失败
        }
        assert!(layout_for(Some("stm32g431rb")).is_some());
        assert!(layout_for(Some("esp32s3")).is_none());
        assert!(layout_for(None).is_none());
        assert!(unknown_layout_note(Some("esp32s3"), 0x3ff4_4000).is_none());
    }
}
