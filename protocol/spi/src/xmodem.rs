//! XModem over SPI (`spec::xmodem`): the module image to BL2 (XModem-1K, `host/core`'s `XmFrame` driver), the MCU's own
//! app to its bootloader (128-byte blocks, `zd1-linkd`'s `mcufw`). The MCU's C side (its receiver in the preloader) is
//! `mcu/src/zd1link.h`, held to this by the generated test.

use crate::spec::xmodem::{BLOCK, BLOCK_1K, SOH, STX};

/// CRC-16/XMODEM (the CCITT polynomial, from `c`: 0 to start): a block's check, and a B&W `.dmb`'s.
pub fn crc16(mut c: u16, d: &[u8]) -> u16 {
    for &b in d {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

/// Block `n` (1-based; its number wraps at 256) of `data` in blocks of `size` (`BLOCK` or `BLOCK_1K`), into `out`
/// (`size` + 5 bytes): SOH or STX, n, ~n, the data (`pad` after its end), the CRC-16 big-endian. The bytes written.
pub fn block(data: &[u8], n: usize, size: u16, pad: u8, out: &mut [u8]) -> usize {
    let size = size as usize;
    debug_assert!(size == BLOCK as usize || size == BLOCK_1K as usize);
    out[0] = if size == BLOCK as usize { SOH } else { STX };
    out[1] = n as u8;
    out[2] = !(n as u8);
    let from = (n - 1) * size;
    let take = data.len().saturating_sub(from).min(size);
    out[3..3 + take].copy_from_slice(&data[from..from + take]);
    out[3 + take..3 + size].fill(pad);
    let c = crc16(0, &out[3..3 + size]);
    out[3 + size..5 + size].copy_from_slice(&c.to_be_bytes());
    size + 5
}

/// A block received (`b`, as `block` writes it): its number, if its header and CRC check.
pub fn check(b: &[u8]) -> Option<u8> {
    let size = match *b.first()? {
        SOH => BLOCK as usize,
        STX => BLOCK_1K as usize,
        _ => return None,
    };
    let ok = b.len() == size + 5
        && b[2] == !b[1]
        && crc16(0, &b[3..3 + size]) == u16::from_be_bytes([b[3 + size], b[4 + size]]);
    ok.then_some(b[1])
}
