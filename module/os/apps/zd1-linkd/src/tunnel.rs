//! The host link from the network (docs/protocol/net.md): zd1-netd hands us a paired client's requests (`host SEQ HEX`
//! on the app socket). The module's own we answer (lib.rs: settings, records); the MCU's go through SPI reg 0xD9, one
//! at a time (the MCU runs one request at a time, from USB or from us): `u16 seq` + the request notified, the MCU
//! writes `u16 seq` + its reply back on the same register. Our sequence numbers, not the client's: two sessions may
//! use the same ones.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

struct Call {
    client: u64,
    seq: u16,
    msg: Vec<u8>,
    /// how long the MCU may take (the request's timeout in the spec, and some)
    wait: Duration,
}

#[derive(Default)]
pub struct Tunnel {
    queue: VecDeque<Call>,
    out: Option<(Call, u16, Instant)>,
    seq: u16,
}

impl Tunnel {
    /// A request of `client` (its `seq`) for the MCU, `wait` the most it may take.
    pub fn push(&mut self, client: u64, seq: u16, msg: Vec<u8>, wait: Duration) {
        self.queue.push_back(Call { client, seq, msg, wait });
    }

    /// The next request to notify (our seq + the message), when none is out.
    pub fn next(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.out.is_some() {
            return None;
        }
        let c = self.queue.pop_front()?;
        self.seq = self.seq.wrapping_add(1);
        let mut p = self.seq.to_le_bytes().to_vec();
        p.extend_from_slice(&c.msg);
        self.out = Some((c, self.seq, now));
        Some(p)
    }

    /// The MCU's reply (its write of 0xD9): whose (client, its seq) and the message. A reply to nothing out (late,
    /// after we gave up) is dropped.
    pub fn reply(&mut self, p: &[u8]) -> Option<(u64, u16, Vec<u8>)> {
        let seq = u16::from_le_bytes([*p.first()?, *p.get(1)?]);
        match &self.out {
            Some((_, s, _)) if *s == seq => {
                let (c, _, _) = self.out.take().unwrap();
                Some((c.client, c.seq, p[2..].to_vec()))
            }
            _ => None,
        }
    }

    /// The request out, if the MCU has had longer than it may take: given up (client, its seq).
    pub fn expired(&mut self, now: Instant) -> Option<(u64, u16)> {
        let late = matches!(&self.out, Some((c, _, t)) if now.duration_since(*t) > c.wait);
        if !late {
            return None;
        }
        let (c, _, _) = self.out.take().unwrap();
        Some((c.client, c.seq))
    }

    /// A client went: its queued requests with it (the one out still gets its reply, dropped then).
    pub fn forget(&mut self, client: u64) {
        self.queue.retain(|c| c.client != client);
        if let Some((c, _, _)) = &mut self.out
            && c.client == client
        {
            c.client = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_at_a_time_in_order() {
        let (mut t, now, w) = (Tunnel::default(), Instant::now(), Duration::from_secs(2));
        t.push(1, 7, vec![0x02], w);
        t.push(2, 7, vec![0x14], w);
        let a = t.next(now).unwrap();
        assert_eq!(&a[2..], [0x02]);
        assert!(t.next(now).is_none(), "one out at a time");
        assert!(t.reply(&[a[0].wrapping_add(1), a[1], 0x42]).is_none(), "not the one out");
        assert_eq!(t.reply(&[a[0], a[1], 0x42]), Some((1, 7, vec![0x42])));
        let b = t.next(now).unwrap();
        assert_ne!(a[..2], b[..2], "our own sequence numbers");
        assert_eq!(t.expired(now + Duration::from_secs(1)), None);
        assert_eq!(t.expired(now + Duration::from_secs(3)), Some((2, 7)));
        assert!(t.reply(&[b[0], b[1], 0x54]).is_none(), "late: dropped");
    }

    #[test]
    fn a_client_that_went() {
        let (mut t, now, w) = (Tunnel::default(), Instant::now(), Duration::from_secs(2));
        t.push(1, 1, vec![0x02], w);
        t.push(1, 2, vec![0x02], w);
        t.push(3, 1, vec![0x02], w);
        let a = t.next(now).unwrap();
        t.forget(1);
        assert_eq!(t.reply(&[a[0], a[1], 0x42]), Some((0, 1, vec![0x42])), "answered to nobody");
        assert!(t.next(now).is_some_and(|p| p[2] == 0x02));
        assert!(t.next(now).is_none(), "client 1's second request went with it");
    }
}
