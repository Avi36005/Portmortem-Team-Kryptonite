//! `HashExpander` (croniter.py:1455) — `H`/`R` expression expansion.
//!
//! Rewrites `h`, `h/15`, `h(0-29)`, `h(30-59)/10` (and the `r` random variants)
//! into ordinary numeric cron syntax before the normal expander runs.
//!
//! The arithmetic must match Python exactly:
//!   crc = binascii.crc32(hash_id) & 0xFFFFFFFF
//!   ((crc >> idx) % (range_end - range_begin + 1)) + range_begin

use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::consts::RANGES;
use crate::error::{CroniterError, Result};

pub static HASH_EXPRESSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<hash_type>h|r)(\((?P<range_begin>\d+)-(?P<range_end>\d+)\))?(/(?P<divisor>\d+))?$",
    )
    .expect("hash_expression_re is a valid regex")
});

/// CRC-32 (IEEE 802.3), identical to Python's `binascii.crc32` / `zlib.crc32`.
///
/// Python returns an unsigned 32-bit value on Python 3, and croniter masks it
/// with `& 0xFFFFFFFF` anyway, so `u32` is exactly the right width.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: LazyLock<[u32; 256]> = LazyLock::new(|| {
        let mut table = [0u32; 256];
        let mut i = 0usize;
        while i < 256 {
            let mut crc = i as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
                bit += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    });

    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Stand-in for Python's `random.randint(0, 0xFFFFFFFF)` in the `r` (random)
/// hash form. A xorshift seeded from the clock keeps `core/` dependency-free;
/// croniter only requires that the value be an arbitrary 32-bit integer.
fn random_u32() -> u32 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = const { Cell::new(0) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x2545_F491_4F6C_DD1D)
                | 1;
        }
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    })
}

/// `HashExpander.do` (croniter.py:1459). Named `hash_do` because `do` is a
/// reserved word in Rust.
pub fn hash_do(
    idx: usize,
    hash_type: &str,
    hash_id: Option<&[u8]>,
    range_begin: Option<i64>,
    range_end: Option<i64>,
) -> i64 {
    let range_end = range_end.unwrap_or(RANGES[idx].1);
    let range_begin = range_begin.unwrap_or(RANGES[idx].0);
    let crc: u64 = if hash_type == "r" {
        u64::from(random_u32())
    } else {
        u64::from(crc32(hash_id.unwrap_or(&[])))
    };
    ((crc >> idx) % ((range_end - range_begin + 1) as u64)) as i64 + range_begin
}

/// `HashExpander.expand` (croniter.py:1474).
///
/// Returns the rewritten field expression, or the input unchanged when it is
/// not a hash expression.
pub fn expand_field(idx: usize, expr: &str, hash_id: Option<&[u8]>) -> Result<String> {
    let caps = match HASH_EXPRESSION_RE.captures(expr) {
        Some(c) => c,
        None => return Ok(expr.to_string()),
    };

    let hash_type = caps.name("hash_type").map(|m| m.as_str()).unwrap_or("h");
    let range_begin = caps.name("range_begin").map(|m| m.as_str());
    let range_end = caps.name("range_end").map(|m| m.as_str());
    let divisor = caps.name("divisor").map(|m| m.as_str());

    if hash_type == "h" && hash_id.is_none() {
        return Err(CroniterError::BadCron(
            "Hashed definitions must include hash_id".to_string(),
        ));
    }

    let parse = |s: &str| -> Result<i64> {
        s.parse::<i64>()
            .map_err(|_| CroniterError::BadCron(format!("Bad expression: {expr}")))
    };

    if let (Some(rb), Some(re_)) = (range_begin, range_end) {
        let (rb, re_) = (parse(rb)?, parse(re_)?);
        if rb >= re_ {
            return Err(CroniterError::BadCron(
                "Range end must be greater than range begin".to_string(),
            ));
        }

        if let Some(div) = divisor {
            // Example: H(30-59)/10 -> 34-59/10 (i.e. 34,44,54)
            let div = parse(div)?;
            if div == 0 {
                return Err(CroniterError::BadCron(format!("Bad expression: {expr}")));
            }
            let x = hash_do(idx, hash_type, hash_id, Some(rb), Some(div - 1 + rb));
            return Ok(format!("{x}-{re_}/{div}"));
        }
        // Example: H(0-29) -> 12
        return Ok(hash_do(idx, hash_type, hash_id, Some(rb), Some(re_)).to_string());
    }

    if let Some(div) = divisor {
        // Example: H/15 -> 7-59/15 (i.e. 7,22,37,52)
        let div = parse(div)?;
        if div == 0 {
            return Err(CroniterError::BadCron(format!("Bad expression: {expr}")));
        }
        let begin = RANGES[idx].0;
        let x = hash_do(idx, hash_type, hash_id, Some(begin), Some(div - 1 + begin));
        return Ok(format!("{x}-{}/{}", RANGES[idx].1, div));
    }

    // Example: H -> 32
    Ok(hash_do(idx, hash_type, hash_id, None, None).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_python_binascii() {
        // Cross-checked against Python: binascii.crc32(b"...") & 0xFFFFFFFF
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"hello"), 0x3610_A686);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn plain_h_is_deterministic_for_a_hash_id() {
        let a = expand_field(0, "h", Some(b"hello")).unwrap();
        let b = expand_field(0, "h", Some(b"hello")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn h_without_hash_id_is_rejected() {
        assert!(expand_field(0, "h", None).is_err());
    }

    #[test]
    fn non_hash_expression_passes_through() {
        assert_eq!(expand_field(0, "*/5", Some(b"x")).unwrap(), "*/5");
        assert_eq!(expand_field(0, "1-5", Some(b"x")).unwrap(), "1-5");
    }

    #[test]
    fn range_end_must_exceed_range_begin() {
        assert!(expand_field(0, "h(30-30)", Some(b"x")).is_err());
        assert!(expand_field(0, "h(40-30)", Some(b"x")).is_err());
    }

    #[test]
    fn zero_divisor_is_rejected() {
        assert!(expand_field(0, "h/0", Some(b"x")).is_err());
        assert!(expand_field(0, "h(0-29)/0", Some(b"x")).is_err());
    }
}
