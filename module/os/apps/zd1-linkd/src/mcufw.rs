//! The MCU's firmware from us, the way the ZD1L's BL2 sends it (docs/mcu/bootloader-hbl.md): the MCU's bootloader
//! (the ZD1L's stock "Net HBL", or our ZD1 preloader that speaks the same) asks the module for BL2 (HCR 0x2120) and
//! takes the app over SPI as XModem. We don't reboot into BL2 for it: zd1-linkd answers as BL2 would, from the image
//! staged in [`PENDING`] (a B&W `.dmb`: 128-byte header, the app for 0x80008000).
//!
//! What the bootloader is told, in order (the emulator trace of the real BL2, re/gen/mcu-bl-zd1l):
//!   status 0x2000, then 0x2300   "rebooting", "BL2 running": it re-reads our versions (its "info ready")
//!   0x29 = 0x2020, 0x2A = 0x2423  HCR status OK; the DCR that ends its DownloadFirmwares and starts its receiver
//!   on 0x2C, XModem blocks        it writes 'C' / ACK / NAK / CAN on 0x2D; 128-byte blocks (SOH), CRC-16, then EOT
//!   0x29 = 0x2020, status 0x2500  the transfer is good; "finished": it powers us off and restarts into the new app.
//!                                 It needs an HCR status OK it has taken after its receive began, and it no longer
//!                                 takes one once the status says finished [EMU 2026-11-19]: the OK goes with the
//!                                 first block and after the EOT, the status a second later
//! The image stays staged until the MCU reports the version it carries (the caller's [`Pending::sent`]).

use std::time::{Duration, Instant};

/// The image the MCU is to get, staged by whoever wants the MCU updated (the system at ready, a host).
pub const PENDING: &str = "/data/zd1/mcu/pending.dmb";

const SOH: u8 = 0x01;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const START: u8 = b'C';
const BLOCK: usize = 128;
pub const DMB_HEADER: usize = 128;

/// HCR status (0x29) and DCR (0x2A) values and statuses (0x11) BL2 uses, as `b0 b1` words.
pub const HCR_OK: u16 = 0x2020;
pub const DCR_HOST_FIRMWARE: u16 = 0x2423;
pub const STATUS_REBOOTING: u16 = 0x2000;
pub const STATUS_BL2: u16 = 0x2300;
pub const STATUS_FINISHED: u16 = 0x2500;

/// A `.dmb`'s app, checked: the header's length and its CRC-16/XMODEM (what the bootloader checks after writing).
pub fn check_dmb(f: &[u8]) -> Result<(), String> {
    if f.len() < DMB_HEADER || f[..4] != [0x00, 0x01, 0x00, 0x80] {
        return Err("not a .dmb (no 00 01 00 80 header)".into());
    }
    let len = u32::from_be_bytes(f[8..12].try_into().unwrap()) as usize;
    if f.len() < DMB_HEADER + len {
        return Err(format!("the header says {len} B of app, the file holds {}", f.len() - DMB_HEADER));
    }
    let want = u16::from_be_bytes(f[16..18].try_into().unwrap());
    let got = crc16(&f[DMB_HEADER..DMB_HEADER + len]);
    if got != want {
        return Err(format!("app CRC-16 {got:04x}, the header says {want:04x}"));
    }
    Ok(())
}

/// CRC-16/XMODEM (CCITT polynomial, init 0): XModem's block check and the `.dmb` header's.
pub fn crc16(d: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &b in d {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

/// XModem block `n` (1-based, wrapping at 256) of `data`: SOH n ~n, 128 bytes (0xFF after the end), CRC-16 big-endian.
pub fn block(data: &[u8], n: usize) -> Vec<u8> {
    let mut b = vec![SOH, n as u8, !(n as u8)];
    let from = (n - 1) * BLOCK;
    let mut chunk = [0xFFu8; BLOCK];
    let take = data.len().saturating_sub(from).min(BLOCK);
    chunk[..take].copy_from_slice(&data[from..from + take]);
    b.extend_from_slice(&chunk);
    b.extend_from_slice(&crc16(&chunk).to_be_bytes());
    b
}

/// What the link should do next: the caller owns the link ([`zd1_link::Link`]).
#[derive(Debug, PartialEq)]
pub enum Out {
    Status(u16),
    /// A notification on a register (0x29, 0x2A as a word; 0x2C a block or EOT).
    Notify(u8, Vec<u8>),
    Say(String),
    /// The transfer is over (true: the bootloader took the whole image).
    Done(bool),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Rebooting,
    Bl2,
    Waiting,
    Sending(usize),
    Eot,
    Finishing,
    Finished,
}

/// One transfer: the image, where it is, and when anything last happened (a bootloader that goes quiet for
/// [`QUIET`] ends it).
pub struct Serve {
    image: Vec<u8>,
    phase: Phase,
    since: Instant,
    last: Instant,
    blocks: usize,
    naks: u32,
    again: u32,
}

const QUIET: Duration = Duration::from_secs(30);
const AGAIN: Duration = Duration::from_secs(7); // the bootloader's read gives up after 5 s
const NAKS: u32 = 10;

impl Serve {
    pub fn new(image: Vec<u8>, now: Instant) -> (Serve, Vec<Out>) {
        let blocks = image.len().div_ceil(BLOCK);
        let s = Serve { image, phase: Phase::Rebooting, since: now, last: now, blocks, naks: 0, again: 0 };
        let say =
            format!("[zd1-linkd] the MCU's bootloader asks for BL2: serving it the staged app ({blocks} blocks)\n");
        (s, vec![Out::Say(say), Out::Status(STATUS_REBOOTING)])
    }

    /// Time passing: the steps that only wait.
    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        let t = now.duration_since(self.since);
        match self.phase {
            Phase::Rebooting if t >= Duration::from_millis(1500) => {
                self.to(Phase::Bl2, now, vec![Out::Status(STATUS_BL2)])
            }
            Phase::Bl2 if t >= Duration::from_secs(3) => self.to(
                Phase::Waiting,
                now,
                vec![
                    Out::Notify(0x29, HCR_OK.to_be_bytes().to_vec()),
                    Out::Notify(0x2A, DCR_HOST_FIRMWARE.to_be_bytes().to_vec()),
                ],
            ),
            Phase::Finishing if t >= Duration::from_secs(1) => {
                self.phase = Phase::Finished;
                vec![Out::Status(STATUS_FINISHED), Out::Done(true)]
            }
            Phase::Finishing | Phase::Finished => vec![],
            // the bootloader read a block it couldn't take (bytes lost: it waits for them, then gives up the read)
            // and waits for another: the same block again, as a notification it reads
            p @ (Phase::Sending(_) | Phase::Eot) if now.duration_since(self.last) > AGAIN && self.again < NAKS => {
                self.again += 1;
                self.last = now;
                let (what, at) = match p {
                    Phase::Sending(n) => (block(&self.image, n), format!("block {n}")),
                    _ => (vec![EOT], "the end".to_string()),
                };
                vec![
                    Out::Say(format!("[zd1-linkd] the MCU's bootloader is quiet at {at}: offered again\n")),
                    Out::Notify(0x2C, what),
                ]
            }
            _ if now.duration_since(self.last) > QUIET => {
                self.phase = Phase::Finished;
                vec![
                    Out::Say("[zd1-linkd] the MCU's bootloader went quiet: transfer abandoned\n".into()),
                    Out::Done(false),
                ]
            }
            _ => vec![],
        }
    }

    /// A control byte from the bootloader (a V write to 0x2D).
    pub fn control(&mut self, c: u8, now: Instant) -> Vec<Out> {
        self.last = now;
        match (self.phase, c) {
            (Phase::Waiting | Phase::Sending(1), START) => {
                self.phase = Phase::Sending(1);
                vec![Out::Notify(0x29, HCR_OK.to_be_bytes().to_vec()), Out::Notify(0x2C, block(&self.image, 1))]
            }
            (Phase::Sending(n), ACK) if n < self.blocks => {
                self.naks = 0;
                self.again = 0;
                self.phase = Phase::Sending(n + 1);
                vec![Out::Notify(0x2C, block(&self.image, n + 1))]
            }
            (Phase::Sending(_), ACK) => {
                self.naks = 0;
                self.phase = Phase::Eot;
                vec![Out::Notify(0x2C, vec![EOT])]
            }
            (Phase::Sending(n), NAK) | (Phase::Sending(n), START) if self.naks < NAKS => {
                self.naks += 1;
                vec![Out::Notify(0x2C, block(&self.image, n))]
            }
            (Phase::Eot, NAK) if self.naks < NAKS => {
                self.naks += 1;
                vec![Out::Notify(0x2C, vec![EOT])]
            }
            (Phase::Eot, ACK) => {
                self.phase = Phase::Finishing;
                self.since = now;
                vec![
                    Out::Say(format!("[zd1-linkd] the MCU took the app ({} blocks)\n", self.blocks)),
                    Out::Notify(0x29, HCR_OK.to_be_bytes().to_vec()),
                ]
            }
            (Phase::Finishing | Phase::Finished, _) => vec![],
            (p, c) => {
                self.phase = Phase::Finished;
                let why = if c == CAN { "cancelled".to_string() } else { format!("0x{c:02x} in {p:?}") };
                vec![Out::Say(format!("[zd1-linkd] MCU transfer ended: {why}\n")), Out::Done(false)]
            }
        }
    }

    fn to(&mut self, p: Phase, now: Instant, out: Vec<Out>) -> Vec<Out> {
        self.phase = p;
        self.since = now;
        self.last = now;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dmb(app: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; DMB_HEADER];
        h[..4].copy_from_slice(&[0, 1, 0, 0x80]);
        h[8..12].copy_from_slice(&(app.len() as u32).to_be_bytes());
        h[12..16].copy_from_slice(&0x8000u32.to_be_bytes());
        h[16..18].copy_from_slice(&crc16(app).to_be_bytes());
        h.extend_from_slice(app);
        h
    }

    #[test]
    fn crc16_is_xmodem() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn a_dmb_is_checked_as_the_bootloader_does() {
        let f = dmb(&[1, 2, 3]);
        assert_eq!(check_dmb(&f), Ok(()));
        let mut bad = f.clone();
        bad[DMB_HEADER] ^= 1;
        assert!(check_dmb(&bad).is_err());
        assert!(check_dmb(&f[..DMB_HEADER + 2]).is_err());
    }

    #[test]
    fn blocks_are_soh_128_crc() {
        let b = block(&[0xAA; 130], 2);
        assert_eq!(&b[..3], &[SOH, 2, 0xFD]);
        assert_eq!(b.len(), 133);
        assert_eq!(&b[3..5], &[0xAA, 0xAA]);
        assert!(b[5..131].iter().all(|&x| x == 0xFF));
    }

    #[test]
    fn a_whole_transfer() {
        let t0 = Instant::now();
        let img = dmb(&[7u8; 200]); // 328 B: 3 blocks
        let (mut s, out) = Serve::new(img.clone(), t0);
        assert!(out.contains(&Out::Status(STATUS_REBOOTING)));
        assert_eq!(s.tick(t0 + Duration::from_secs(2)), vec![Out::Status(STATUS_BL2)]);
        let w = s.tick(t0 + Duration::from_secs(6));
        assert_eq!(w[1], Out::Notify(0x2A, vec![0x24, 0x23]));
        let now = t0 + Duration::from_secs(7);
        assert_eq!(s.control(START, now), vec![Out::Notify(0x29, vec![0x20, 0x20]), Out::Notify(0x2C, block(&img, 1))]);
        assert_eq!(s.control(NAK, now), vec![Out::Notify(0x2C, block(&img, 1))]);
        assert_eq!(s.control(ACK, now), vec![Out::Notify(0x2C, block(&img, 2))]);
        assert_eq!(s.control(ACK, now), vec![Out::Notify(0x2C, block(&img, 3))]);
        assert_eq!(s.control(ACK, now), vec![Out::Notify(0x2C, vec![EOT])]);
        assert_eq!(s.control(NAK, now), vec![Out::Notify(0x2C, vec![EOT])]);
        let end = s.control(ACK, now);
        assert!(end.contains(&Out::Notify(0x29, vec![0x20, 0x20])) && !end.contains(&Out::Status(STATUS_FINISHED)));
        assert_eq!(s.tick(now + Duration::from_millis(500)), vec![]);
        assert_eq!(s.tick(now + Duration::from_secs(2)), vec![Out::Status(STATUS_FINISHED), Out::Done(true)]);
    }

    #[test]
    fn a_block_is_offered_again_when_the_bootloader_goes_quiet() {
        let t0 = Instant::now();
        let img = dmb(&[7u8; 200]);
        let (mut s, _) = Serve::new(img.clone(), t0);
        s.tick(t0 + Duration::from_secs(2));
        s.tick(t0 + Duration::from_secs(6));
        let now = t0 + Duration::from_secs(7);
        s.control(START, now);
        s.control(ACK, now);
        let again = s.tick(now + Duration::from_secs(8));
        assert!(again.contains(&Out::Notify(0x2C, block(&img, 2))));
        assert_eq!(s.control(ACK, now + Duration::from_secs(9)), vec![Out::Notify(0x2C, block(&img, 3))]);
    }

    #[test]
    fn a_cancel_or_silence_ends_it() {
        let t0 = Instant::now();
        let (mut s, _) = Serve::new(dmb(&[0; 10]), t0);
        s.tick(t0 + Duration::from_secs(2));
        s.tick(t0 + Duration::from_secs(6));
        assert!(s.control(CAN, t0 + Duration::from_secs(7)).contains(&Out::Done(false)));
        let (mut s, _) = Serve::new(dmb(&[0; 10]), t0);
        s.tick(t0 + Duration::from_secs(2));
        s.tick(t0 + Duration::from_secs(6));
        assert!(s.tick(t0 + Duration::from_secs(60)).contains(&Out::Done(false))); // waiting for 'C': no block to offer
    }
}
