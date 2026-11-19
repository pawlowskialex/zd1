//! Network settings changed over the network are provisional (docs/protocol/net.md): a change that can cut the client
//! off the speaker (`net::provisional`: an interface's address, the Wi-Fi's networks, the defaults back) keeps what
//! was there before it, and the client that made it has `net::CONFIRM_S` to reach the speaker again (a new session,
//! at the new address or the old one, opened `net::SETTLE_S` or more after the change: by then it has taken effect).
//! It doesn't: the settings and the known networks from before come back, and the services that use them start
//! again. One client's changes at a time.

use std::time::{Duration, Instant};
use zd1_hostlink::net;

#[derive(Default)]
pub struct Guard {
    pending: Option<Pending>,
}

struct Pending {
    client: [u8; 32],
    since: Instant,
    until: Instant,
    keys: Vec<String>,
    /// the files as they were (None: there was none)
    before: Vec<(&'static str, Option<Vec<u8>>)>,
}

/// What a provisional change touches.
const FILES: &[&str] = &[zd1_settings::FILE, zd1_settings::NETWORKS];

impl Guard {
    /// `client` is about to change `key` over the network: what is there now kept (the first change of theirs), the
    /// clock started again. Err: another client's change waits for its confirmation.
    pub fn before(&mut self, client: &[u8; 32], key: &str, now: Instant) -> Result<(), String> {
        let until = now + Duration::from_secs(net::CONFIRM_S as u64);
        match &mut self.pending {
            Some(p) if &p.client != client => {
                return Err("another client's network change waits for it to connect again".into());
            }
            Some(p) => {
                p.since = now;
                p.until = until;
                p.keys.push(key.to_string());
            }
            None => {
                let before = FILES.iter().map(|f| (*f, std::fs::read(f).ok())).collect();
                self.pending =
                    Some(Pending { client: *client, since: now, until, keys: vec![key.to_string()], before });
            }
        }
        crate::say(&format!(
            "{key} changed over the network: kept once that client connects again within {} s",
            net::CONFIRM_S
        ));
        Ok(())
    }

    /// `client` opened a session (its handshake done `at`): its changes made before are kept, once they have taken
    /// effect.
    pub fn connected(&mut self, client: &[u8; 32], at: Instant) {
        let settled = |p: &Pending| at >= p.since + Duration::from_secs(net::SETTLE_S as u64);
        if self.pending.as_ref().is_some_and(|p| &p.client == client && settled(p)) {
            let p = self.pending.take().unwrap();
            crate::say(&format!("the client is back: {} kept", p.keys.join(", ")));
        }
    }

    /// Time's up without the client: the settings from before, back.
    pub fn tick(&mut self, now: Instant) {
        if !self.pending.as_ref().is_some_and(|p| now >= p.until) {
            return;
        }
        let p = self.pending.take().unwrap();
        crate::say(&format!("the client didn't connect again: {} back as before", p.keys.join(", ")));
        for (f, b) in &p.before {
            let r = match b {
                Some(b) => zd1_sys::write_private(f, &String::from_utf8_lossy(b)).map_err(|e| e.to_string()),
                None => match std::fs::remove_file(f) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                    _ => Ok(()),
                },
            };
            if let Err(e) = r {
                crate::say(&format!("{f}: {e}"));
            }
        }
        for k in &p.keys {
            zd1_settings::apply(k); // what uses it starts again
        }
    }
}
