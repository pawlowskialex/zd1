//! A B&W over-the-air `.fw` (the Lightning model's first install, through its BL2's web updater;
//! docs/mcu/stock-firmware.md): a 32-byte wrapper (u32 little-endian: version 2, the `.bcd`'s length, 0 = not
//! encrypted, the MCU part's length, a CRC-32, the release, 0, 0), the module's `.bcd`, the MCU's part (a `.dmb`).
//! `tools/zeppelin/fw/bwfw.py` packs one.

/// The `.bcd` and the MCU part of `d`, if it is an unencrypted `.fw`.
pub fn parts(d: &[u8]) -> Option<(&[u8], &[u8])> {
    let w = |i: usize| d.get(4 * i..4 * i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    if w(0)? != 2 || w(2)? != 0 {
        return None;
    }
    let (b, m) = (w(1)?, w(3)?);
    let bcd = d.get(32..32 + b)?;
    let mcu = d.get(32 + b..32 + b + m)?;
    bcd.starts_with(b"bCoD").then_some((bcd, mcu))
}
