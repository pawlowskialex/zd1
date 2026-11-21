//! The files the flows take, told apart by [`identify`]. MCU firmware: Intel HEX (absolute addresses), a raw .bin of
//! the app region (it starts at 0x80004000: the build's zd1.bin and a backup's app_flash.bin alike) or a backup
//! (`backup`: its app region), identified by the block mcu/stamp.py writes (`ZD1-FW-IDENT-01`). The module's system:
//! a gzip'd newc cpio of a slot's root, identified by its usr/lib/os-release; its stamp is "VERSION SHA256[:12]
//! LENGTH". The module's boot image: a BridgeCo `.bcd` (docs/module/boot-chain.md).

use crate::backup::{self, Backup};
use crate::spec::Model;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// The MCU's flash: at 0x80000000, 256 KB; the bootloader the first 16 KB (BOOTPROT: never written).
pub const FLASH_BASE: u32 = 0x8000_0000;
pub const FLASH_SIZE: u32 = 0x4_0000;
pub const BOOTLOADER_END: u32 = 0x4000;
/// The user page (0x80800000, 512 bytes): the ISP word at 0x1FC.
pub const USER_PAGE_SIZE: u32 = 0x200;
/// The known-good ISP configuration word (CRC-8 and signature): without it the bootloader enters DFU at every boot.
pub const ISP_WORD: u32 = 0x929E_0870;

/// An MCU image: bytes by absolute address.
#[derive(Clone, Debug, Default)]
pub struct McuImage {
    pub mem: BTreeMap<u32, u8>,
}

/// What mcu/stamp.py put in an image: its version (the app's content, `-dirty` with uncommitted changes) and the
/// commit's date (`zd1-release`).
pub use zd1_release::Ident;
/// The identification in `data` (one block exactly, else None).
pub use zd1_release::ident::find as ident;

impl McuImage {
    /// Intel HEX text, a backup (its app region), or else a raw image of the app region (at 0x80004000).
    pub fn parse(name: &str, data: &[u8]) -> Result<McuImage> {
        if name.to_ascii_lowercase().ends_with(".hex") || data.first() == Some(&b':') {
            McuImage::from_hex(data)
        } else if backup::is_tar(data) {
            Ok(McuImage::from_bin(FLASH_BASE + BOOTLOADER_END, &Backup::from_tar(data)?.app))
        } else {
            Ok(McuImage::from_bin(FLASH_BASE + BOOTLOADER_END, data))
        }
    }

    pub fn from_bin(base: u32, data: &[u8]) -> McuImage {
        McuImage { mem: data.iter().enumerate().map(|(i, b)| (base.wrapping_add(i as u32), *b)).collect() }
    }

    /// Intel HEX: data records, extended segment and linear addresses, the end record.
    pub fn from_hex(text: &[u8]) -> Result<McuImage> {
        let (mut base, mut mem) = (0u32, BTreeMap::new());
        for (n, l) in text.split(|c| *c == b'\n').enumerate() {
            let bad = |what: &str| Error::Failed(format!("hex line {}: {what}", n + 1));
            let Some(h) = l.trim_ascii().strip_prefix(b":") else { continue };
            let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
            let b = h
                .chunks(2)
                .map(|p| match p {
                    [a, b] => Some(digit(*a)? << 4 | digit(*b)?),
                    _ => None,
                })
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| bad("not hex"))?;
            if b.len() < 5 || b.len() != 5 + b[0] as usize || b.iter().fold(0u8, |a, x| a.wrapping_add(*x)) != 0 {
                return Err(bad("bad length or checksum"));
            }
            let (addr, data) = (u16::from_be_bytes([b[1], b[2]]) as u32, &b[4..4 + b[0] as usize]);
            let word = || match data {
                [hi, lo] => Ok(u16::from_be_bytes([*hi, *lo]) as u32),
                _ => Err(bad("an address record of two bytes")),
            };
            match b[3] {
                0 => {
                    for (i, x) in data.iter().enumerate() {
                        mem.insert(base.wrapping_add(addr + i as u32), *x);
                    }
                }
                1 => break,
                2 => base = word()? << 4,
                4 => base = word()? << 16,
                _ => {}
            }
        }
        Ok(McuImage { mem })
    }

    /// The app region's span in flash offsets: [start, end), start on a 1 KB block (the DFU's unit). Refused: an
    /// image with nothing there, or with bytes outside the flash (below it, or past its end).
    pub fn app_span(&self) -> Result<(u32, u32)> {
        let (Some((&lo, _)), Some((&first, _)), Some((&last, _))) = (
            self.mem.range(FLASH_BASE + BOOTLOADER_END..).next(),
            self.mem.first_key_value(),
            self.mem.last_key_value(),
        ) else {
            return Err(Error::Failed("the image has nothing in the MCU's app region".into()));
        };
        if first < FLASH_BASE || last >= FLASH_BASE + FLASH_SIZE {
            return Err(Error::Failed(format!(
                "the image has bytes at 0x{:08x}, outside the MCU's flash (0x{FLASH_BASE:08x}..0x{:08x})",
                if first < FLASH_BASE { first } else { last },
                FLASH_BASE + FLASH_SIZE
            )));
        }
        Ok(((lo - FLASH_BASE) & !(crate::dfu::BLOCK - 1), last + 1 - FLASH_BASE))
    }

    /// Bytes in the bootloader's region (never written: ignored).
    pub fn in_bootloader(&self) -> usize {
        self.mem.range(..FLASH_BASE + BOOTLOADER_END).count()
    }

    /// Flash offsets [start, end) as bytes, 0xFF where the image has none.
    pub fn bytes(&self, start: u32, end: u32) -> Vec<u8> {
        (start..end).map(|a| *self.mem.get(&(FLASH_BASE + a)).unwrap_or(&0xFF)).collect()
    }

    /// The identification in the app region.
    pub fn ident(&self) -> Option<Ident> {
        let (s, e) = self.app_span().ok()?;
        ident(&self.bytes(s, e))
    }
}

/// What a file is.
pub enum Kind {
    /// MCU firmware (Intel HEX, the app region's .bin); its identification if it is ours.
    Mcu(McuImage),
    /// A backup of the MCU (`backup`).
    Backup(Backup),
    /// The module's system: its version and the stamp its slot gets.
    System { version: String, stamp: String },
    /// The module's system signed for the network (`sign_system`): what its header says, and the key's that signed it
    /// is the speaker's to check.
    Signed { version: String, built: String, stamp: String },
    /// A B&W over-the-air `.fw` (the first install on a Zeppelin Air with Lightning or an A7, through its BL2's web
    /// updater; docs/mcu/stock-firmware.md): the model its `.bcd`'s HWID is for (BL2 takes it only on that model),
    /// the module's `.bcd` (ours: its identity) and the MCU's app (ours: its version).
    Fw { model: Model, bcd_ours: bool, boot: String, mcu: Option<Ident> },
    /// The module's boot image (a BridgeCo `.bcd`): its build string (ours keeps the stock container's), whether it
    /// boots our OS (its stage 2 is our boot stub, module/os/mkpayload.py) or the stock app, and ours's identity
    /// (`zd1.boot=` on its kernel command line, as the State record's `boot` reports it; empty: none).
    Bcd { build: String, ours: bool, boot: String },
}

/// Which of the files the flows take `data` is (`name` for a HEX file without its first `:`), refused if it is none.
pub fn identify(name: &str, data: &[u8]) -> Result<Kind> {
    if data.starts_with(&[0x1F, 0x8B]) {
        return Ok(Kind::System { version: system_version(data)?, stamp: system_stamp(data)? });
    }
    if data.starts_with(zd1_release::MAGIC.as_bytes()) {
        let (h, _) = zd1_release::Header::parse(data).map_err(Error::Failed)?;
        return Ok(Kind::Signed { stamp: h.stamp(), version: h.version, built: h.built });
    }
    if let Some((bcd, mcu)) = zd1_release::fw::parts(data) {
        let ours = bcd_ours(bcd);
        return Ok(Kind::Fw {
            model: bcd_model(bcd),
            bcd_ours: ours,
            boot: if ours { bcd_boot(bcd) } else { String::new() },
            mcu: ident(mcu),
        });
    }
    if data.starts_with(b"bCoD") {
        let build = data.get(8..0x18).map_or(String::new(), |b| String::from_utf8_lossy(b).trim().to_string());
        let ours = bcd_ours(data);
        return Ok(Kind::Bcd { build, ours, boot: if ours { bcd_boot(data) } else { String::new() } });
    }
    if backup::is_tar(data) {
        return Ok(Kind::Backup(Backup::from_tar(data)?));
    }
    let img = McuImage::parse(name, data)?;
    img.app_span()?;
    Ok(Kind::Mcu(img))
}

/// A `.bcd` whose stage 2 (0x2000 into its first section, docs/module/boot-chain.md) starts with our boot stub's
/// trampoline and its 'ZLB1' header at 0x40 (module/os/mkpayload.py).
/// The model a `.bcd` is for: its header's HWID (+0x20, the low 16 bits; docs/module/boot-chain.md), which BL2's web
/// updater holds against the speaker's (0xBD the Zeppelin Air, 0xBE with Lightning, 0xA7 the A7).
pub fn bcd_model(data: &[u8]) -> Model {
    match data.get(0x20..0x22).map(|b| u16::from_le_bytes([b[0], b[1]])) {
        Some(0xBD) => Model::Zd1,
        Some(0xBE) => Model::Zd1l,
        Some(0xA7) => Model::A7,
        _ => Model::Unknown,
    }
}

/// A model's short name, as release files and the page's manifest name it (`zd1l.fw`, `a7.fw`).
pub fn model_key(m: Model) -> &'static str {
    match m {
        Model::Zd1 => "zd1",
        Model::Zd1l => "zd1l",
        Model::A7 => "a7",
        _ => "unknown",
    }
}

fn bcd_ours(data: &[u8]) -> bool {
    let Some(sec) = data.get(0x30..0x34).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else {
        return false;
    };
    let at = sec.saturating_add(0x2000 + 0x40);
    data.get(at..at.saturating_add(4)) == Some(b"ZLB1")
}

/// Our boot image's identity: `zd1.boot=ID` on the kernel command line, which the payload carries as text
/// (module/os/assemble.py, payload).
fn bcd_boot(data: &[u8]) -> String {
    const KEY: &[u8] = b"zd1.boot=";
    let Some(at) = data.windows(KEY.len()).position(|w| w == KEY) else {
        return String::new();
    };
    let id = &data[at + KEY.len()..];
    let n = id.iter().take(16).take_while(|b| b.is_ascii_hexdigit()).count();
    String::from_utf8_lossy(&id[..n]).into_owned()
}

/// gzip, inflated (its CRC not checked: the update checks the SHA-256 of the whole).
pub fn gunzip(gz: &[u8]) -> Result<Vec<u8>> {
    let bad = || Error::Failed("not a gzip file".into());
    if gz.len() < 18 || gz[0] != 0x1F || gz[1] != 0x8B || gz[2] != 8 {
        return Err(bad());
    }
    let flags = gz[3];
    let mut i = 10;
    if flags & 4 != 0 {
        i += 2 + u16::from_le_bytes([gz[i], gz[i + 1]]) as usize;
    } // FEXTRA
    for bit in [8u8, 16] {
        // FNAME, FCOMMENT
        if flags & bit != 0 {
            i += gz.get(i..).and_then(|r| r.iter().position(|c| *c == 0)).ok_or_else(bad)? + 1;
        }
    }
    if flags & 2 != 0 {
        i += 2;
    } // FHCRC
    miniz_oxide::inflate::decompress_to_vec(gz.get(i..).ok_or_else(bad)?)
        .map_err(|e| Error::Failed(format!("gzip: {e:?}")))
}

/// A file in a newc cpio archive.
pub fn cpio_file<'a>(cpio: &'a [u8], path: &str) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + 110 <= cpio.len() && &cpio[i..i + 6] == b"070701" {
        let f = |k: usize| {
            std::str::from_utf8(&cpio[i + 6 + 8 * k..i + 14 + 8 * k])
                .ok()
                .and_then(|s| usize::from_str_radix(s, 16).ok())
        };
        let (size, nsize) = (f(6)?, f(11)?);
        let name = cpio.get(i + 110..i + 110 + nsize.saturating_sub(1))?;
        let mut d = i + 110 + nsize;
        d += (4 - d % 4) % 4;
        if name == path.as_bytes() {
            return cpio.get(d..d + size);
        }
        if name == b"TRAILER!!!" {
            break;
        }
        i = d + size;
        i += (4 - i % 4) % 4;
    }
    None
}

/// A system image's version: VERSION_ID of its usr/lib/os-release ("unknown" without one).
pub fn system_version(image: &[u8]) -> Result<String> {
    Ok(system_release(image)?.0)
}

/// A system image's version and build date: VERSION_ID and BUILD_DATE of its usr/lib/os-release ("unknown" and ""
/// without them).
pub fn system_release(image: &[u8]) -> Result<(String, String)> {
    let cpio = gunzip(image)?;
    let text =
        cpio_file(&cpio, "usr/lib/os-release").map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('=')).map(|v| v.trim_matches('"').to_string()))
    };
    Ok((get("VERSION_ID").unwrap_or_else(|| "unknown".into()), get("BUILD_DATE").unwrap_or_default()))
}

/// A system image signed for the network (`protocol/release`, docs/protocol/net.md): its header, made from the
/// image's version and build date and signed with `seed` (a signing key), then the image.
pub fn sign_system(image: &[u8], seed: &[u8; 32]) -> Result<Vec<u8>> {
    if image.starts_with(zd1_release::MAGIC.as_bytes()) {
        return Err(Error::Failed("signed already".into()));
    }
    let (version, built) = system_release(image)?;
    let mut out = zd1_release::sign(image, &version, &built, seed).map_err(Error::Failed)?;
    out.extend_from_slice(image);
    Ok(out)
}

/// A signed system update's header and where its image starts (None: not one).
pub fn signed_system(update: &[u8]) -> Option<(zd1_release::Header, usize)> {
    update.starts_with(zd1_release::MAGIC.as_bytes()).then(|| zd1_release::Header::parse(update).ok()).flatten()
}

/// The stamp a slot gets (what `zd1-ctl status` shows): "VERSION SHA256[:12] LENGTH".
pub fn system_stamp(image: &[u8]) -> Result<String> {
    Ok(zd1_release::stamp(&system_version(image)?, &crate::sha256(image), image.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_ident() {
        let mut blk = zd1_release::ident::MAGIC.to_vec();
        blk.extend(b"abc1234\0".iter().chain(std::iter::repeat_n(&0, 33)));
        blk.extend(b"2026-10-30 12:00\0".iter().chain(std::iter::repeat_n(&0, 8)));
        let img = McuImage::from_bin(0x8000_4000, &[vec![0xAA; 100], blk.clone()].concat());
        assert_eq!(img.app_span(), Ok((0x4000, 0x4000 + 100 + blk.len() as u32)));
        assert_eq!(img.ident(), Some(Ident { version: "abc1234".into(), built: "2026-10-30 12:00".into() }));
        // the same as Intel HEX: an extended linear address, 16-byte records
        let mut text = String::from(":0200000480007A\n");
        for (k, c) in img.bytes(0x4000, 0x4000 + 100 + blk.len() as u32).chunks(16).enumerate() {
            let a = 0x4000 + 16 * k as u32;
            let mut r = vec![c.len() as u8, (a >> 8) as u8, a as u8, 0];
            r.extend(c);
            r.push(r.iter().fold(0u8, |s, x| s.wrapping_sub(*x)));
            text += &format!(":{}\n", r.iter().map(|x| format!("{x:02X}")).collect::<String>());
        }
        text += ":00000001FF\n";
        let h = McuImage::parse("x.hex", text.as_bytes()).unwrap();
        assert_eq!(h.mem, img.mem);
        assert!(McuImage::from_hex(b":0100000000FE\n").is_err()); // a bad checksum
        assert!(McuImage::from_hex(":02000004é0007A\n".as_bytes()).is_err()); // not hex, without a panic
        assert!(McuImage::from_hex(b":0000000400FC\n").is_err()); // an address record without its address
        // past the flash's end, and nothing in the app region
        assert!(McuImage::from_bin(0x8003_FFF0, &[0; 32]).app_span().is_err());
        assert!(McuImage::from_bin(0x8000_0000, &[0; 16]).app_span().is_err());
    }

    #[test]
    fn system_image() {
        fn entry(name: &str, data: &[u8]) -> Vec<u8> {
            let mut h = format!(
                "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
                0,
                0o100644,
                0,
                0,
                1,
                0,
                data.len(),
                0,
                0,
                0,
                0,
                name.len() + 1,
                0
            )
            .into_bytes();
            h.extend(name.as_bytes());
            h.push(0);
            while h.len() % 4 != 0 {
                h.push(0);
            }
            h.extend(data);
            while h.len() % 4 != 0 {
                h.push(0);
            }
            h
        }
        let cpio = [
            entry("etc", b""),
            entry("usr/lib/os-release", b"NAME=zd1-os\nVERSION_ID=\"e674e93\"\n"),
            entry("TRAILER!!!", b""),
        ]
        .concat();
        let mut gz = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3];
        gz.extend(miniz_oxide::deflate::compress_to_vec(&cpio, 6));
        gz.extend([0; 8]);
        assert_eq!(system_version(&gz).unwrap(), "e674e93");
        assert!(system_stamp(&gz).unwrap().starts_with("e674e93 "));
        assert!(system_version(b"not gzip at all, no").is_err());
    }
}
