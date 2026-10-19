//! 探针库的独立冒烟程序（不经过 agent、不经过基准）。
//!
//! ```text
//! cargo run -p firment-tools --example probe_smoke
//! cargo run -p firment-tools --example probe_smoke -- --elf path/to/firmware.elf
//! cargo run -p firment-tools --example probe_smoke -- --rtt-ms 5000
//! ```
//!
//! 为什么要有它：`debug mem` / `flash` / `run` 是工具，跑它们要起 agent、给权限、
//! 等一次模型往返。而"库到底能不能 attach / 读到什么 / 烧多久 / RTT 通不通"
//! 是纯粹的硬件问题，应该一条命令就能回答。
//!
//! 每一步都打印耗时，失败也照样往下走 —— 它是一次体检，不是一个测试套件。

use std::time::{Duration, Instant};

use firment_tools::probe;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut chip = "stm32g431rb".to_string();
    let mut elf: Option<String> = None;
    let mut rtt_ms: u64 = 0;
    let mut addr: u64 = 0x4800_0800; // GPIOC 基址
    let mut words: usize = 6;
    let mut ops: Vec<&'static str> = Vec::new();

    while let Some(a) = args.next() {
        match a.as_str() {
            "--chip" => chip = args.next().unwrap_or(chip),
            "--elf" => elf = args.next(),
            "--rtt-ms" => rtt_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--addr" => {
                addr = args
                    .next()
                    .and_then(|v| {
                        let v = v.trim_start_matches("0x");
                        u64::from_str_radix(v, 16).ok()
                    })
                    .unwrap_or(addr)
            }
            "--words" => words = args.next().and_then(|v| v.parse().ok()).unwrap_or(words),
            "--halt" => ops.push("halt"),
            "--step" => ops.push("step"),
            "--resume" => ops.push("resume"),
            "--help" | "-h" => {
                println!(
                    "usage: probe_smoke [--chip <id>] [--elf <file>] [--rtt-ms <n>] [--addr <hex>] [--words <n>]"
                );
                return;
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
        }
    }

    println!("chip: {chip}");

    // 1. 枚举
    let t = Instant::now();
    let n = probe::probes_visible();
    println!(
        "[{}] probes visible: {n} ({} ms)",
        if n > 0 { "ok" } else { "FAIL" },
        t.elapsed().as_millis()
    );
    if n == 0 {
        println!("no probe attached — attach one and re-run");
        return;
    }

    // 2. 读
    let t = Instant::now();
    match probe::read_words(&chip, None, addr, words) {
        Ok(w) => {
            let hex: Vec<String> = w.iter().map(|v| format!("{v:08x}")).collect();
            println!(
                "[ok] read {words} words @ {addr:#010x}: {} ({} ms)",
                hex.join(" "),
                t.elapsed().as_millis()
            );
        }
        Err(e) => println!(
            "[FAIL] read: {} ({} ms)",
            e.message(),
            t.elapsed().as_millis()
        ),
    }

    // 3. 写（写回读到的同一个值：证明可写，又不改状态）
    if let Ok(w) = probe::read_words(&chip, None, addr, 1) {
        let t = Instant::now();
        match probe::write_word(&chip, None, addr, w[0]) {
            Ok(()) => println!(
                "[ok] wrote {:#x} back to {addr:#010x} ({} ms)",
                w[0],
                t.elapsed().as_millis()
            ),
            Err(e) => println!(
                "[FAIL] write: {} ({} ms)",
                e.message(),
                t.elapsed().as_millis()
            ),
        }
    }

    // 3.5 核操作（halt / step / resume）
    for op in &ops {
        let which = match *op {
            "halt" => probe::CoreOp::Halt,
            "step" => probe::CoreOp::Step,
            _ => probe::CoreOp::Resume,
        };
        let t = Instant::now();
        match probe::core_op(&chip, None, which) {
            Ok(()) => println!("[ok] {op} ({} ms)", t.elapsed().as_millis()),
            Err(e) => println!(
                "[FAIL] {op}: {} ({} ms)",
                e.message(),
                t.elapsed().as_millis()
            ),
        }
    }

    // 4. 烧
    if let Some(path) = &elf {
        let t = Instant::now();
        match probe::flash_elf(&chip, None, std::path::Path::new(path)) {
            Ok(segs) => println!(
                "[ok] flashed {segs} segment(s) from {path} ({} ms)",
                t.elapsed().as_millis()
            ),
            Err(e) => println!(
                "[FAIL] flash: {} ({} ms)",
                e.message(),
                t.elapsed().as_millis()
            ),
        }
    } else {
        println!("[skip] flash (no --elf)");
    }

    // 5. RTT
    if rtt_ms > 0 {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let t = Instant::now();
        let mut bytes = 0usize;
        let res = probe::rtt_stream(&chip, None, Duration::from_millis(rtt_ms), &stop, |chunk| {
            bytes += chunk.len();
            print!("{}", String::from_utf8_lossy(chunk));
        });
        match res {
            Ok(_) => println!("\n[ok] RTT {bytes} bytes in {} ms", t.elapsed().as_millis()),
            Err(e) => println!(
                "[FAIL] RTT: {} ({} ms)",
                e.message(),
                t.elapsed().as_millis()
            ),
        }
    } else {
        println!("[skip] RTT (--rtt-ms 0)");
    }
}
