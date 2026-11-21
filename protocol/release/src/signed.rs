//! The signed system update (docs/protocol/net.md, System updates over the network): a system image behind a text
//! header that names it, signed with Ed25519. Our OS takes one over the network only when a key it trusts signed the
//! header (`/etc/zd1/keys`, `/data/zd1/keys`); the header binds the image by its length and SHA-256, so an image
//! that isn't the signed one is refused at its end, before the slot is stamped.
//!
//! ```text
//! zd1-system-update 1
//! version abc1234                the image's VERSION_ID (its /usr/lib/os-release)
//! built 2026-11-19 18:55         its BUILD_DATE: the OS takes nothing older than what it runs
//! length 5312345                 the image's bytes
//! sha256 <64 hex>                their SHA-256
//! signature <128 hex>            Ed25519 over every byte of the header before this line
//!                                an empty line, then the image
//! ```
//! Keys are files of one line: the 32-byte key in hex (a public key; a signing key holds the seed), `#` comments.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use zd1_text::{hex, unhex_n};

pub const MAGIC: &str = "zd1-system-update 1";
/// The longest header we read: the fields are short.
pub const HEADER_MAX: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: String,
    pub built: String,
    pub length: u64,
    pub sha256: [u8; 32],
    pub signature: [u8; 64],
    /// The bytes the signature covers (the header up to its `signature` line).
    signed: Vec<u8>,
}

impl Header {
    /// The header at the start of `b`, and its length (the image follows); Err: not one, or not whole within
    /// [`HEADER_MAX`].
    pub fn parse(b: &[u8]) -> Result<(Header, usize), String> {
        let end = b
            .windows(2)
            .take(HEADER_MAX)
            .position(|w| w == b"\n\n")
            .ok_or("not a signed system update (no header within 1 KB)")?;
        let text = std::str::from_utf8(&b[..end + 1]).map_err(|_| "a header that isn't text")?;
        let mut lines = text.split_inclusive('\n');
        if lines.next().map(str::trim_end) != Some(MAGIC) {
            return Err(format!("not a signed system update (no \"{MAGIC}\")"));
        }
        let (mut version, mut built, mut length, mut sha256, mut signature, mut signed) =
            (None, None, None, None, None, None);
        let mut at = MAGIC.len() + 1;
        for l in lines {
            let (k, v) = l.trim_end().split_once(' ').ok_or_else(|| format!("header line {:?}", l.trim_end()))?;
            match k {
                "version" => version = Some(v.to_string()),
                "built" => built = Some(v.to_string()),
                "length" => length = Some(v.parse::<u64>().map_err(|_| "length: not a number")?),
                "sha256" => sha256 = Some(unhex_n::<32>(v).ok_or("sha256: not 64 hex digits")?),
                "signature" => {
                    signature = Some(unhex_n::<64>(v).ok_or("signature: not 128 hex digits")?);
                    signed = Some(b[..at].to_vec());
                }
                _ => {} // a field a newer signer adds: signed, not read
            }
            if signature.is_some() && k != "signature" {
                return Err("a field after the signature".into());
            }
            at += l.len();
        }
        let missing = |f: &str| format!("the header has no {f}");
        Ok((
            Header {
                version: version.ok_or_else(|| missing("version"))?,
                built: built.ok_or_else(|| missing("built"))?,
                length: length.ok_or_else(|| missing("length"))?,
                sha256: sha256.ok_or_else(|| missing("sha256"))?,
                signature: signature.ok_or_else(|| missing("signature"))?,
                signed: signed.unwrap(),
            },
            end + 2,
        ))
    }

    /// Which of `keys` signed it (its index); Err: none did.
    pub fn verify(&self, keys: &[[u8; 32]]) -> Result<usize, String> {
        if keys.is_empty() {
            return Err("no keys to check it against".into());
        }
        let sig = Signature::from_bytes(&self.signature);
        keys.iter()
            .position(|k| VerifyingKey::from_bytes(k).is_ok_and(|k| k.verify(&self.signed, &sig).is_ok()))
            .ok_or_else(|| "signed by none of the keys this speaker trusts".into())
    }

    /// The slot's stamp it gets ([`crate::stamp`]).
    pub fn stamp(&self) -> String {
        crate::stamp(&self.version, &self.sha256, self.length)
    }
}

/// The header for `image` (its version and build date, from its os-release), signed with `seed` (a signing key).
/// The update is the header followed by the image.
pub fn sign(image: &[u8], version: &str, built: &str, seed: &[u8; 32]) -> Result<Vec<u8>, String> {
    for (f, v) in [("version", version), ("built", built)] {
        if v.is_empty() || v.contains('\n') {
            return Err(format!("{f}: one line, not empty"));
        }
    }
    let mut h = format!(
        "{MAGIC}\nversion {version}\nbuilt {built}\nlength {}\nsha256 {}\n",
        image.len(),
        hex(&Sha256::digest(image))
    )
    .into_bytes();
    let sig = SigningKey::from_bytes(seed).sign(&h);
    h.extend_from_slice(format!("signature {}\n\n", hex(&sig.to_bytes())).as_bytes());
    Ok(h)
}

/// The public key of a signing key.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// A key file's key: the first line that isn't a comment, 64 hex digits.
pub fn read_key(text: &str) -> Option<[u8; 32]> {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).and_then(unhex_n::<32>)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    #[test]
    fn signed_and_checked() {
        let image = b"an image".to_vec();
        let h = sign(&image, "abc1234", "2026-11-19 18:55", &SEED).unwrap();
        let mut b = h.clone();
        b.extend(&image);
        let (p, n) = Header::parse(&b).unwrap();
        assert_eq!(n, h.len());
        assert_eq!(&b[n..], &image[..]);
        assert_eq!((p.version.as_str(), p.built.as_str(), p.length), ("abc1234", "2026-11-19 18:55", 8));
        assert_eq!(p.sha256, <[u8; 32]>::from(Sha256::digest(&image)));
        let other = public_key(&[8; 32]);
        assert_eq!(p.verify(&[other, public_key(&SEED)]), Ok(1));
        assert!(p.verify(&[other]).is_err());
        assert!(p.verify(&[]).is_err());
        assert_eq!(p.stamp(), format!("abc1234 {} 8", &hex(&p.sha256)[..12]));
    }

    #[test]
    fn a_changed_header_fails() {
        let h = sign(b"x", "abc1234", "2026-11-19 18:55", &SEED).unwrap();
        let t = String::from_utf8(h).unwrap();
        for (from, to) in [("built 2026-11-19", "built 2027-11-19"), ("length 1", "length 2"), ("abc1234", "abc1235")] {
            let (p, _) = Header::parse(t.replace(from, to).as_bytes()).unwrap();
            assert!(p.verify(&[public_key(&SEED)]).is_err(), "{from} -> {to}");
        }
        // a field slipped in after the signature, or none at all
        assert!(Header::parse(t.replace("\n\n", "\nversion evil\n\n").as_bytes()).is_err());
        assert!(Header::parse(b"zd1-system-update 1\nversion a\n\n").is_err());
        assert!(Header::parse(b"\x1f\x8b\x08\x00").is_err());
    }

    #[test]
    fn keys() {
        let k = public_key(&SEED);
        assert_eq!(read_key(&format!("# the release key\n{}\n", hex(&k))), Some(k));
        assert_eq!(read_key("# nothing\n"), None);
        assert_eq!(read_key("abcd"), None);
    }
}
