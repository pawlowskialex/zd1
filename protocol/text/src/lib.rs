//! Text that the Rust on both sides writes and reads, once: the module's programs, the computer's (`zd1`), the page's
//! WebAssembly, and the crates they share. Hex, a UTC date, a string cut to a field's bytes. No dependencies.
#![no_std]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(2 * b.len());
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

/// Hex (either case) back to bytes; None for an odd length or a character that isn't a hex digit.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let d = |c: u8| (c as char).to_digit(16);
    b.chunks(2).map(|p| Some((d(p[0])? * 16 + d(p[1])?) as u8)).collect()
}

/// Exactly `N` bytes of hex (a key: 32 bytes in 64 digits), surrounding white space ignored.
pub fn unhex_n<const N: usize>(s: &str) -> Option<[u8; N]> {
    unhex(s.trim())?.try_into().ok()
}

/// A key's short name in messages and lists (the network's paired clients, the update keys): its first 8 hex digits.
pub fn key_id(key: &[u8]) -> String {
    hex(&key[..key.len().min(4)])
}

/// `s` cut to at most `max` bytes, at a character's boundary (a field's maximum).
pub fn cut(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// A moment as UTC's calendar has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Utc {
    /// Seconds since 1970-01-01 00:00 UTC (Howard Hinnant's civil_from_days).
    pub fn from_unix(secs: i64) -> Utc {
        let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let z = days + 719_468;
        let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let (day, month) = ((doy - (153 * mp + 2) / 5 + 1) as u32, if mp < 10 { mp + 3 } else { mp - 9 } as u32);
        let (s, year) = (s as u32, yoe + era * 400 + i64::from(month <= 2));
        Utc { year, month, day, hour: s / 3600, minute: s / 60 % 60, second: s % 60 }
    }

    /// `2026-11-19T20:15:03Z`
    pub fn iso(&self) -> String {
        let Utc { year, month, day, hour, minute, second } = self;
        let mut s = String::new();
        let _ = write!(s, "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
        s
    }

    /// `2026-11-19 20:15`
    pub fn minutes(&self) -> String {
        let Utc { year, month, day, hour, minute, .. } = self;
        let mut s = String::new();
        let _ = write!(s, "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_both_ways() {
        assert_eq!(hex(&[0x00, 0xAB, 0x7f]), "00ab7f");
        assert_eq!(unhex("00AB7f"), Some(alloc::vec![0x00, 0xAB, 0x7F]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex_n::<2>(" 0102\n"), Some([1, 2]));
        assert_eq!(unhex_n::<2>("010203"), None);
    }

    #[test]
    fn cut_at_a_boundary() {
        assert_eq!(cut("Küche", 2), "K");
        assert_eq!(cut("Küche", 3), "Kü");
        assert_eq!(cut("abc", 10), "abc");
    }

    #[test]
    fn dates() {
        assert_eq!(Utc::from_unix(0).iso(), "1970-01-01T00:00:00Z");
        assert_eq!(Utc::from_unix(951_782_400).minutes(), "2000-02-29 00:00"); // a leap day
        assert_eq!(Utc::from_unix(1_795_119_303).iso(), "2026-11-19T20:15:03Z");
        assert_eq!(Utc::from_unix(-1).iso(), "1969-12-31T23:59:59Z");
    }
}
