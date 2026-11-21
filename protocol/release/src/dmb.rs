//! A B&W `.dmb`: the MCU's app as the bootloaders take it (the ZD1L's "Net HBL", our ZD1 preloader;
//! docs/mcu/bootloader-hbl.md), and what our system carries for the MCU and serves it. Big-endian: `00 01 00 80`, the
//! build time, +8 the app's length, +12 its place in flash, +16 its CRC-16/XMODEM, +24 a name, then the app at +128.
//! `tools/zeppelin/fw/bwfw.py` makes them; the MCU's C reads the header too (`mcu/src/mcufw.h`).

use zd1_spi::xmodem::crc16;

pub const MAGIC: [u8; 4] = [0x00, 0x01, 0x00, 0x80];
pub const HEADER: usize = 128;

/// The app in `f`, checked as the bootloader checks it after writing: the header's length and CRC-16.
pub fn app(f: &[u8]) -> Result<&[u8], String> {
    if f.len() < HEADER || f[..4] != MAGIC {
        return Err("not a .dmb (no 00 01 00 80 header)".into());
    }
    let len = u32::from_be_bytes(f[8..12].try_into().unwrap()) as usize;
    let app = f
        .get(HEADER..HEADER + len)
        .ok_or_else(|| format!("the header says {len} B of app, the file holds {}", f.len() - HEADER))?;
    let (want, got) = (u16::from_be_bytes(f[16..18].try_into().unwrap()), crc16(0, app));
    if got != want {
        return Err(format!("app CRC-16 {got:04x}, the header says {want:04x}"));
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked() {
        let app = [0x5Au8; 300];
        let mut f = vec![0u8; super::HEADER];
        f[..4].copy_from_slice(&super::MAGIC);
        f[8..12].copy_from_slice(&300u32.to_be_bytes());
        f[16..18].copy_from_slice(&zd1_spi::xmodem::crc16(0, &app).to_be_bytes());
        f.extend(app);
        assert_eq!(super::app(&f).map(<[u8]>::len), Ok(300));
        let mut bad = f.clone();
        bad[super::HEADER] ^= 1;
        assert!(super::app(&bad).is_err());
        assert!(super::app(&f[..super::HEADER + 2]).is_err());
        assert!(super::app(&[0u8; 200]).is_err());
    }
}
