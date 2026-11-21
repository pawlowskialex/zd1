//! The files a release of ours is made of, read and written once for every side that handles them (the computer's
//! `zd1` and page, our module OS): the signed system update ([`signed`]), the MCU's app as a B&W `.dmb` ([`dmb`]), the
//! identification `mcu/stamp.py` writes into the MCU's firmware ([`ident`]), the Lightning model's B&W `.fw`
//! ([`fw`]), and the stamp a system slot gets ([`stamp`]). Python's tools (`tools/zeppelin/fw`) build them.

pub mod dmb;
pub mod fw;
pub mod ident;
pub mod signed;

pub use ident::Ident;
pub use signed::{HEADER_MAX, Header, MAGIC, public_key, read_key, sign};

/// What a system slot is stamped with (`/.zd1-system`, `zd1-ctl status`, the State record's `stamp_a`/`stamp_b`):
/// "VERSION SHA256[:12] LENGTH", the image's version, the start of its SHA-256 in hex, its length.
pub fn stamp(version: &str, sha256: &[u8; 32], length: u64) -> String {
    format!("{version} {} {length}", &zd1_text::hex(sha256)[..12])
}
