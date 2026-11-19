//! What `mcu/stamp.py` writes into our MCU firmware after the build (`mcu/src/version.c`'s block): a magic, the
//! version (the commit, `-dirty` with uncommitted changes) and its date, found by the magic in any image of it (the
//! app, the region, a `.dmb`, a backup).

/// The block's magic; the version's field (41 bytes) and the date's (25) follow it, NUL-padded.
pub const MAGIC: &[u8] = b"ZD1-FW-IDENT-01\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub version: String,
    pub built: String,
}

/// The identification in `data` (one block exactly, else None).
pub fn find(data: &[u8]) -> Option<Ident> {
    let at = |h: &[u8]| h.windows(MAGIC.len()).position(|w| w == MAGIC);
    let i = at(data)?;
    if at(&data[i + 1..]).is_some() {
        return None;
    }
    let p = i + MAGIC.len();
    let field = |a: usize, n: usize| {
        let b = data.get(a..a + n)?;
        Some(String::from_utf8_lossy(b.split(|c| *c == 0).next().unwrap_or(b)).into_owned())
    };
    Some(Ident { version: field(p, 41)?, built: field(p + 41, 25)? })
}

#[cfg(test)]
mod tests {
    #[test]
    fn found_once() {
        let mut b = vec![0u8; 300];
        b.extend(super::MAGIC);
        b.extend(b"abc1234\0".iter().chain(std::iter::repeat_n(&0, 33)));
        b.extend(b"2026-10-30 12:00\0".iter().chain(std::iter::repeat_n(&0, 8)));
        let i = super::find(&b).unwrap();
        assert_eq!((i.version.as_str(), i.built.as_str()), ("abc1234", "2026-10-30 12:00"));
        assert_eq!(super::find(&[0u8; 64]), None);
        let twice = [b.clone(), b].concat();
        assert_eq!(super::find(&twice), None, "two blocks: not one image");
    }
}
