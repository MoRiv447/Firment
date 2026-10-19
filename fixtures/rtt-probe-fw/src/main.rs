//! RTT 夹具固件。
//!
//! 它只做一件事：初始化 RTT 并每秒打印一行带序号的字符串。
//! `probe_smoke --rtt-ms 3000` 能读到递增的序号，就说明 `rtt_stream`
//! 走的是真 RTT，而不是"attach 成功但没有数据"。

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;
use rtt_target::{rprintln, rtt_init_print};

#[entry]
fn main() -> ! {
    rtt_init_print!();
    let mut n: u32 = 0;
    loop {
        rprintln!("firment-rtt-smoke {}", n);
        n = n.wrapping_add(1);
        // 约 1 秒的忙等，够慢到能看清"字节是分批到的"。
        for _ in 0..8_000_000 {
            cortex_m::asm::nop();
        }
    }
}
