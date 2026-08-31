//! The deterministic mutation engine — the reproducibility floor of the
//! red team.
//!
//! Given a seed, a baseline message and a strategy list, `corpus()` returns
//! the SAME byte sequence of cases every run, on every machine, with no
//! rand crate and no LLM. That is what makes a red-team finding a REGRESSION
//! TEST: the reproducer is `seed + case id`, not "ask the model again and
//! hope". The LLM attacker (campaign phase) explores on top of this corpus;
//! anything it finds must be back-ported into a corpus case to enter the
//! report.
//!
//! Strategies are protocol-agnostic byte mutations of the baseline frame —
//! boundary lengths, single-bit flips, oversize payloads, format strings,
//! delimiter confusion and numeric extremes.

use serde::Deserialize;

/// Small fast deterministic PRNG (the standard SplitMix64). No dependency,
/// no platform float/iterator-order variance: the corpus is byte-identical
/// across runs by construction.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A mutation family the suite may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mutation {
    /// Length and value boundaries: empty, 1 byte, 2^n±1 lengths, all-0x00,
    /// all-0xFF, first byte forced to extremes.
    Boundary,
    /// Single-bit flips. Every bit for short baselines (<= 16 bytes); a
    /// seeded sample for longer ones.
    Bitflip,
    /// 2x, 16x and 64 KiB payloads — buffer and length-field traps.
    Oversize,
    /// Classic format-string probes appended to the baseline.
    Format,
    /// Delimiter confusion: NUL, CRLF, repeated separators, truncated tail.
    Delimiter,
    /// ASCII numeric extremes (-1, 4294967295, NaN, oversized digits).
    Numeric,
}

/// One generated test case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// Stable identifier: `{strategy}-{index}` — the reproducer key.
    pub id: String,
    pub strategy: Mutation,
    pub payload: Vec<u8>,
    /// Human-readable intent, surfaced in the report.
    pub desc: String,
}

/// Above this baseline length, bitflip switches from exhaustive to a seeded
/// sample (256 flips is already a lot of cases for a 16-byte frame).
const BITFLIP_EXHAUSTIVE_MAX: usize = 16;
/// Sampled flips for longer baselines.
const BITFLIP_SAMPLED: usize = 64;

/// Build the deterministic corpus. Cases appear in strategy order, then in
/// index order within each strategy — the iteration order IS the contract
/// (replay depends on it).
pub fn corpus(seed: u64, baseline: &[u8], strategies: &[Mutation]) -> Vec<Case> {
    let mut cases = Vec::new();
    for strat in strategies {
        match strat {
            Mutation::Boundary => cases.extend(boundary_cases(baseline)),
            Mutation::Bitflip => cases.extend(bitflip_cases(seed, baseline)),
            Mutation::Oversize => cases.extend(oversize_cases(baseline)),
            Mutation::Format => cases.extend(format_cases(baseline)),
            Mutation::Delimiter => cases.extend(delimiter_cases(baseline)),
            Mutation::Numeric => cases.extend(numeric_cases(baseline)),
        }
    }
    let mut counters = [0usize; 6];
    for case in cases.iter_mut() {
        counters[strategy_index(case.strategy)] += 1;
        case.id = format!(
            "{}-{}",
            strategy_name(case.strategy),
            counters[strategy_index(case.strategy)]
        );
    }
    cases
}

fn strategy_index(m: Mutation) -> usize {
    match m {
        Mutation::Boundary => 0,
        Mutation::Bitflip => 1,
        Mutation::Oversize => 2,
        Mutation::Format => 3,
        Mutation::Delimiter => 4,
        Mutation::Numeric => 5,
    }
}

pub fn strategy_name(m: Mutation) -> &'static str {
    match m {
        Mutation::Boundary => "boundary",
        Mutation::Bitflip => "bitflip",
        Mutation::Oversize => "oversize",
        Mutation::Format => "format",
        Mutation::Delimiter => "delimiter",
        Mutation::Numeric => "numeric",
    }
}

fn case(strategy: Mutation, payload: Vec<u8>, desc: impl Into<String>) -> Case {
    Case {
        id: String::new(),
        strategy,
        payload,
        desc: desc.into(),
    }
}

fn boundary_cases(b: &[u8]) -> Vec<Case> {
    let mut out = vec![
        case(Mutation::Boundary, vec![], "empty frame"),
        case(
            Mutation::Boundary,
            b.first().copied().map_or(vec![0x00], |x| vec![x]),
            "single byte (truncated header)",
        ),
    ];
    // Length boundaries around the baseline: 2^n - 1 / 2^n / 2^n + 1.
    let len = b.len();
    if len > 0 {
        for target in [len.saturating_sub(1), len + 1, len + 2] {
            let mut v = b.to_vec();
            v.resize(target, v.last().copied().unwrap_or(0x00));
            out.push(case(
                Mutation::Boundary,
                v,
                format!("length {target} (baseline {len} ± boundary)"),
            ));
        }
    }
    for pow in [1u32, 2, 3, 4, 5] {
        let n = 1usize << pow; // 2..32
        if n != len && n <= 64 {
            let mut v = b.to_vec();
            v.resize(n, b.last().copied().unwrap_or(0x00));
            out.push(case(
                Mutation::Boundary,
                v,
                format!("length {n} (power-of-two boundary)"),
            ));
        }
    }
    out.push(case(Mutation::Boundary, vec![0x00; len.max(1)], "all 0x00"));
    out.push(case(Mutation::Boundary, vec![0xFF; len.max(1)], "all 0xFF"));
    if len > 0 {
        let mut first0 = b.to_vec();
        first0[0] = 0x00;
        out.push(case(Mutation::Boundary, first0, "first byte 0x00"));
        let mut first_ff = b.to_vec();
        first_ff[0] = 0xFF;
        out.push(case(Mutation::Boundary, first_ff, "first byte 0xFF"));
    }
    out
}

fn bitflip_cases(seed: u64, b: &[u8]) -> Vec<Case> {
    let bits = b.len() * 8;
    if bits == 0 {
        return Vec::new();
    }
    let mut rng = SplitMix64::new(seed ^ 0xBD_F1_1F);
    if bits <= BITFLIP_EXHAUSTIVE_MAX * 8 {
        (0..bits)
            .map(|bit| {
                let mut v = b.to_vec();
                v[bit / 8] ^= 1 << (bit % 8);
                case(
                    Mutation::Bitflip,
                    v,
                    format!("flip bit {bit} (byte {}/{})", bit / 8, b.len()),
                )
            })
            .collect()
    } else {
        // Seeded sample: deterministic given (seed, bits).
        let mut picked: Vec<usize> = (0..BITFLIP_SAMPLED)
            .map(|_| (rng.next_u64() % bits as u64) as usize)
            .collect();
        picked.sort_unstable();
        picked.dedup();
        picked
            .into_iter()
            .map(|bit| {
                let mut v = b.to_vec();
                v[bit / 8] ^= 1 << (bit % 8);
                case(
                    Mutation::Bitflip,
                    v,
                    format!("flip bit {bit} (sampled, byte {}/{})", bit / 8, b.len()),
                )
            })
            .collect()
    }
}

fn oversize_cases(b: &[u8]) -> Vec<Case> {
    let mut out = Vec::new();
    for (mult, label) in [(2, "2x"), (16, "16x")] {
        let mut v = Vec::with_capacity(b.len() * mult);
        for _ in 0..mult {
            v.extend_from_slice(b);
        }
        out.push(case(
            Mutation::Oversize,
            v,
            format!("{label} baseline length"),
        ));
    }
    let mut big = Vec::with_capacity(64 * 1024);
    while big.len() < 64 * 1024 {
        if b.is_empty() {
            big.push(0x41);
        } else {
            big.extend_from_slice(b);
            big.truncate(64 * 1024);
        }
    }
    out.push(case(Mutation::Oversize, big, "64 KiB payload"));
    out
}

fn format_cases(b: &[u8]) -> Vec<Case> {
    const PROBES: [&[u8]; 5] = [b"%s%n%x", b"%n", b"%s%s%s%s%s%s", b"{}", b"%p%p%p%p"];
    PROBES
        .iter()
        .map(|p| {
            let mut v = b.to_vec();
            v.extend_from_slice(p);
            case(
                Mutation::Format,
                v,
                format!("append format probe {:?}", String::from_utf8_lossy(p)),
            )
        })
        .collect()
}

fn delimiter_cases(b: &[u8]) -> Vec<Case> {
    let mut out = Vec::new();
    for (suffix, label) in [
        (&b"\r\n\0"[..], "CRLF + NUL"),
        (&b"\0"[..], "NUL"),
        (&b"::"[..], "double colon"),
        (&b"\n\n\n"[..], "repeated newlines"),
        (&b"%%"[..], "doubled percent"),
    ] {
        let mut v = b.to_vec();
        v.extend_from_slice(suffix);
        out.push(case(Mutation::Delimiter, v, format!("append {label}")));
    }
    if b.len() > 1 {
        // Unterminated frame: drop the last byte (often the checksum or the
        // closing delimiter).
        out.push(case(
            Mutation::Delimiter,
            b[..b.len() - 1].to_vec(),
            "truncated last byte (unterminated frame)",
        ));
    }
    out
}

fn numeric_cases(b: &[u8]) -> Vec<Case> {
    const TOKENS: &[&str] = &[
        "-1",
        "2147483647",
        "-2147483648",
        "4294967295",
        "99999999999999999999",
        "NaN",
        "0x7fffffff",
        "0xFFFFFFFFFFFFFFFF",
    ];
    TOKENS
        .iter()
        .map(|t| {
            let mut v = b.to_vec();
            v.extend_from_slice(t.as_bytes());
            case(
                Mutation::Numeric,
                v,
                format!("append numeric extreme '{t}'"),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn all_strategies() -> Vec<Mutation> {
        vec![
            Mutation::Boundary,
            Mutation::Bitflip,
            Mutation::Oversize,
            Mutation::Format,
            Mutation::Delimiter,
            Mutation::Numeric,
        ]
    }

    #[test]
    fn same_seed_same_corpus_byte_for_byte() {
        let baseline = [0x55, 0xAA, 0x01, 0x02];
        let a = corpus(1234, &baseline, &all_strategies());
        let b = corpus(1234, &baseline, &all_strategies());
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.payload, y.payload);
        }
    }

    #[test]
    fn different_seed_changes_the_sampled_bitflips() {
        let long = [0x11u8; 32]; // 256 bits > exhaustive max
        let a = corpus(1, &long, &[Mutation::Bitflip]);
        let b = corpus(2, &long, &[Mutation::Bitflip]);
        assert_ne!(
            a.iter().map(|c| &c.payload).collect::<Vec<_>>(),
            b.iter().map(|c| &c.payload).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bitflips_have_hamming_distance_one() {
        let baseline = [0x55, 0xAA, 0x01, 0x02];
        for c in corpus(7, &baseline, &[Mutation::Bitflip]) {
            assert_eq!(c.payload.len(), baseline.len());
            let diff = c
                .payload
                .iter()
                .zip(&baseline)
                .map(|(a, b)| (a ^ b).count_ones())
                .sum::<u32>();
            assert_eq!(diff, 1, "{} must differ by exactly one bit", c.desc);
        }
    }

    #[test]
    fn exhaustive_bitflip_covers_every_bit_of_a_short_frame() {
        let baseline = [0x00, 0x00];
        let cases = corpus(7, &baseline, &[Mutation::Bitflip]);
        assert_eq!(cases.len(), 16);
    }

    #[test]
    fn oversize_lengths_are_exact() {
        let baseline = [0xAB; 10];
        let cases = corpus(7, &baseline, &[Mutation::Oversize]);
        let lens: Vec<usize> = cases.iter().map(|c| c.payload.len()).collect();
        assert_eq!(lens, vec![20, 160, 64 * 1024]);
    }

    #[test]
    fn ids_are_stable_and_scoped_per_strategy() {
        let cases = corpus(7, &[0x01, 0x02], &all_strategies());
        assert_eq!(cases[0].id, "boundary-1");
        let first_bitflip = cases
            .iter()
            .find(|c| c.strategy == Mutation::Bitflip)
            .unwrap();
        assert_eq!(first_bitflip.id, "bitflip-1");
        // No duplicate ids.
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn empty_baseline_does_not_panic() {
        let cases = corpus(7, &[], &all_strategies());
        assert!(
            !cases.is_empty(),
            "boundary/oversize/format still produce cases"
        );
        assert!(cases.iter().all(|c| !c.id.is_empty()));
    }

    #[test]
    fn strategy_order_is_the_given_order() {
        let cases = corpus(7, &[0x01], &[Mutation::Numeric, Mutation::Boundary]);
        assert_eq!(cases.first().unwrap().strategy, Mutation::Numeric);
        let first_boundary = cases
            .iter()
            .position(|c| c.strategy == Mutation::Boundary)
            .unwrap();
        assert!(
            cases
                .iter()
                .take(first_boundary)
                .all(|c| c.strategy == Mutation::Numeric),
            "numeric block must come first"
        );
    }

    #[test]
    fn mutation_names_deserialize_lowercase() {
        #[derive(Deserialize)]
        struct W {
            strategies: Vec<Mutation>,
        }
        let w: W = toml::from_str(
            r#"strategies = ["boundary", "bitflip", "oversize", "format", "delimiter", "numeric"]"#,
        )
        .unwrap();
        assert_eq!(w.strategies, all_strategies());
        assert!(toml::from_str::<W>(r#"strategies = ["BitFlip"]"#).is_err());
    }

    #[test]
    fn splitmix64_is_a_deterministic_scrambler() {
        let mut a = SplitMix64::new(0);
        let mut b = SplitMix64::new(0);
        let mut c = SplitMix64::new(1);
        let seq_a: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        let seq_c: Vec<u64> = (0..4).map(|_| c.next_u64()).collect();
        assert_eq!(seq_a, seq_b, "same seed, same sequence");
        assert_ne!(seq_a, seq_c, "different seed, different sequence");
        assert!(
            seq_a.windows(2).all(|w| w[0] != w[1]),
            "no immediate repeats"
        );
    }
}
