//! 最小的 ITM 包解码器。
//!
//! 为什么自己写：`probe-rs` 0.32 只提供 SWO 的**传输**（`SwoAccess`），
//! ITM 的包解析在 `probe-rs-tools` 那个二进制里，不对外；crates.io 上的 `itm`
//! 最新只到 0.3.1，是**目标端**的发包宏，不含主机端解码。
//!
//! 范围刻意很小：只解**源头包**（stimulus port 的 printf 输出）、溢出、以及
//! 时间戳类包里能数出个数的那些；其余包按字节跳过并计数。一个"看懂 printf"
//! 的解码器值 80 行，一个完整的 ITM 栈不值。
//!
//! 规范：ARMv7-M 架构参考手册 C1.10（ITM 包格式）。

/// 解码出来的一条输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItmLine {
    /// stimulus port（0 = 通常是 printf）。
    pub port: u8,
    pub text: String,
}

/// 解码统计：让"没收到数据"和"收到了但都是时间戳"分得开。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ItmStats {
    pub sync_bytes: usize,
    pub source_packets: usize,
    pub overflow_packets: usize,
    pub timestamp_packets: usize,
    pub unknown_bytes: usize,
}

/// 增量解码器。
///
/// SWO 是**字节流**：一个包可能跨两次 read。所以这里攒一个缓冲区，只在凑齐
/// 一个完整包时才消费；剩下的尾巴留到下一次 `push`。
#[derive(Debug, Default)]
pub struct ItmDecoder {
    buf: Vec<u8>,
    stats: ItmStats,
    /// 每条 stimulus 端口的行缓冲：包是字节片断，换行才是一条"输出"。
    lines: [String; 32],
}

/// 一个包的解析结果。
enum Step {
    /// 消费了 n 字节，没有可打印内容。
    Skip(usize),
    /// 一串字节要写进某个端口的行缓冲。
    Bytes(u8, Vec<u8>),
    /// 数据不够，等下一次 push。
    NeedMore,
}

impl ItmDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> ItmStats {
        self.stats
    }

    /// 喂一段字节，返回这段里**完整**的行。
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ItmLine> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.buf.len() {
            match parse_one(&self.buf[i..]) {
                Step::NeedMore => break,
                Step::Skip(n) => {
                    match self.buf[i] {
                        0x00 => self.stats.sync_bytes += 1,
                        0x70 => self.stats.overflow_packets += 1,
                        _ => self.stats.timestamp_packets += 1,
                    }
                    i += n;
                }
                Step::Bytes(port, data) => {
                    self.stats.source_packets += 1;
                    for b in data {
                        self.feed(port, b, &mut out);
                    }
                    // parse_one 已经把整个包算进 n 里，这里按包头长度推进。
                    i += 1 + data_len(&self.buf[i]);
                }
            }
        }
        self.buf.drain(..i);
        out
    }

    fn feed(&mut self, port: u8, byte: u8, out: &mut Vec<ItmLine>) {
        let idx = (port as usize) % self.lines.len();
        if byte == b'\n' || byte == 0 {
            let line = std::mem::take(&mut self.lines[idx]);
            if !line.is_empty() {
                out.push(ItmLine { port, text: line });
            }
            return;
        }
        if byte.is_ascii_control() && byte != b'\t' {
            return;
        }
        let s = &mut self.lines[idx];
        if s.len() < 4096 {
            s.push(byte as char);
        }
    }

    /// 收尾：把没换行的半行也吐出来（设备掉线时常见）。
    pub fn flush(&mut self) -> Vec<ItmLine> {
        let mut out = Vec::new();
        for port in 0..self.lines.len() {
            let line = std::mem::take(&mut self.lines[port]);
            if !line.is_empty() {
                out.push(ItmLine {
                    port: port as u8,
                    text: line,
                });
            }
        }
        out
    }
}

/// 源头包的负载长度：bit1 = 0 → 1 字节，1 → 4 字节。
fn data_len(header: &u8) -> usize {
    if header & 0b10 != 0 { 4 } else { 1 }
}

/// 解析一个包。只有 `NeedMore` 表示"数据没到齐"。
fn parse_one(bytes: &[u8]) -> Step {
    let header = bytes[0];
    if header == 0x00 {
        return Step::Skip(1);
    }
    // 溢出包先判：0x70 与"port 14 的单字节源头包"字节相同，ARM 把它定义为溢出。
    // 代价是端口 14 的单字节形式读不出来 —— 真实 printf 走 4 字节包，不受影响。
    if header == 0x70 {
        return Step::Skip(1);
    }
    // 源头包：bit0 = 0，bit2 = 0（保留），其余位是端口与宽度。
    if header & 0b0000_0001 == 0 && header & 0b0000_0100 == 0 {
        let port = header >> 3;
        let n = data_len(&header);
        if bytes.len() < 1 + n {
            return Step::NeedMore;
        }
        return Step::Bytes(port, bytes[1..1 + n].to_vec());
    }
    if header == 0x70 {
        return Step::Skip(1);
    }
    // 其余（时间戳、扩展）：按已知长度跳过，长度不够就等。
    let skip = match header {
        0x78 => 2,
        0x80..=0x87 => 3,
        0x88..=0x8F | 0xA0..=0xBF | 0xC0..=0xDF => 2,
        _ => 1,
    };
    if bytes.len() < skip {
        return Step::NeedMore;
    }
    Step::Skip(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_byte_source_packet_carries_one_char() {
        let mut d = ItmDecoder::new();
        // port 1（头字节 0b0000_1000）。**不能用 port 0**：它的 1 字节头正好是
        // 0x00，与 ITM 的同步/空闲字节相同 —— 这也是真实 printf 走 4 字节包的原因。
        let lines = d.push(&[0b0000_1000, b'h']);
        assert!(lines.is_empty(), "no newline yet");
        let lines = d.push(&[0b0000_1000, b'\n']);
        assert_eq!(
            lines,
            vec![ItmLine {
                port: 1,
                text: "h".into()
            }]
        );
    }

    #[test]
    fn a_zero_byte_is_sync_not_a_port_zero_packet() {
        let mut d = ItmDecoder::new();
        let lines = d.push(&[0x00, 0x00, 0x00]);
        assert!(lines.is_empty());
        assert_eq!(d.stats().sync_bytes, 3);
        assert_eq!(d.stats().source_packets, 0);
    }

    #[test]
    fn a_line_split_across_reads_reassembles() {
        let mut d = ItmDecoder::new();
        for b in b"hello" {
            d.push(&[0x08, *b]);
        }
        let lines = d.push(&[0x08, b'\n']);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello");
    }

    #[test]
    fn wide_packets_are_little_endian_and_skip_padding() {
        let mut d = ItmDecoder::new();
        // port 0，4 字节负载：'A' 'B' '\n' 0  —— 小端写法是 0x00_0A_42_41
        let v: u32 = 0x000A_4241;
        let mut pkt = vec![0b0000_0010u8];
        pkt.extend_from_slice(&v.to_le_bytes());
        let lines = d.push(&pkt);
        assert_eq!(
            lines,
            vec![ItmLine {
                port: 0,
                text: "AB".into()
            }]
        );
    }

    #[test]
    fn stimulus_ports_are_kept_apart() {
        let mut d = ItmDecoder::new();
        d.push(&[(1 << 3), b'x']);
        d.push(&[(2 << 3), b'y']);
        let lines = d.push(&[(1 << 3), b'\n']);
        assert_eq!(lines[0].port, 1);
        assert_eq!(lines[0].text, "x");
        let more = d.flush();
        assert_eq!(more[0].port, 2);
        assert_eq!(more[0].text, "y");
    }

    #[test]
    fn overflow_is_counted_not_printed() {
        let mut d = ItmDecoder::new();
        d.push(&[0x70]);
        assert_eq!(d.stats().overflow_packets, 1);
    }

    #[test]
    fn a_truncated_packet_waits_for_the_rest() {
        let mut d = ItmDecoder::new();
        // 4 字节包头 + 只到 2 字节负载
        let r = d.push(&[0b0000_0010, b'a', b'b']);
        assert!(r.is_empty());
        // 补上剩余两字节
        let r = d.push(b"c\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "abc");
    }
}
