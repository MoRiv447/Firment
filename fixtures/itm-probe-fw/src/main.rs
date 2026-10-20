//! ITM 夹具固件：只做一件事 —— 打开 stimulus port 0 并持续打印。
//!
//! 它存在的理由与 `rtt-probe-fw` 相同：`trace` 的 SWO 分支没有固件能验证。
//! 这里不引 `itm` crate，直接写寄存器 —— 要验的就是"裸写能不能被解出来"，
//! 多一层宏只会让失败点变模糊。
//!
//! 两处必要的使能：
//! - `ITM_TER` bit0：打开 stimulus port 0（否则写 STIM 寄存器什么也不会发生）。
//! - `DBGMCU_CR` bit5 `TRACE_IOEN` + TRACE_MODE=00：让 TPIU 的引脚进入异步跟踪模式
//!   （STM32 特有；probe-rs 的 `enable_swo` 只配 TPIU 本体，不管这个开关）。

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const ITM_TER: *mut u32 = 0xE000_0E00 as *mut u32;
const ITM_STIM0: *mut u32 = 0xE000_0000 as *mut u32;
const DBGMCU_CR: *mut u32 = 0xE004_2004 as *mut u32;

/// 一次写 4 字节（port 0 的 4 字节包）。不足 4 字节用 0 补齐，
/// 解码器把 0 当行尾/丢弃 —— 所以这里每行都按 4 的倍数发。
fn emit(bytes: &[u8]) {
    let mut i = 0;
    while i < bytes.len() {
        let mut word = [0u8; 4];
        let n = (bytes.len() - i).min(4);
        word[..n].copy_from_slice(&bytes[i..i + n]);
        let v = u32::from_le_bytes(word);
        unsafe { core::ptr::write_volatile(ITM_STIM0, v) };
        i += 4;
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // TRACE_IOEN=1（bit5），TRACE_MODE=00（bit6:7，异步）
        let cr = core::ptr::read_volatile(DBGMCU_CR);
        core::ptr::write_volatile(DBGMCU_CR, cr | 0x20);
        // 打开 stimulus port 0
        core::ptr::write_volatile(ITM_TER, core::ptr::read_volatile(ITM_TER) | 1);
    }

    let mut n: u32 = 0;
    loop {
        emit(b"itm-smoke 0000\n");
        // 把序号写成四位十进制，避免依赖格式化（`core::fmt` 会拉进大量代码）。
        let mut buf = [b'0'; 4];
        let mut v = n % 10_000;
        for i in (0..4).rev() {
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        emit(&buf);
        emit(b"\n");
        n = n.wrapping_add(1);
        for _ in 0..2_000_000 {
            cortex_m::asm::nop();
        }
    }
}
