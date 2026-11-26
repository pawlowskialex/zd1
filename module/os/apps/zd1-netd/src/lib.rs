//! zd1-netd: the network link (docs/protocol/net.md). Paired clients (the web page, `zd1 --net`) reach the speaker's
//! host link over the LAN: HTTP on port 80, `GET /zd1` upgraded to a WebSocket, a Noise_XX session inside it, the host
//! link's messages inside that. A client the speaker hasn't paired may say `Hello` and `Pair` (ON/OFF held 5 s on
//! the speaker); a paired one makes the requests the spec marks `net`. We answer `Pair` and `SystemUpdate`; the rest
//! goes to zd1-linkd (`host SEQ HEX`), which answers the module's own (settings, records) and passes the MCU's to it.
//! One thread per connection, at most `net::SESSIONS`.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use zd1_hostlink::net;
use zd1_text::key_id;

mod guard;
mod linkd;
mod session;
mod update;

/// A line for the log (init hands our stdout to it).
pub(crate) fn say(s: &str) {
    println!("[zd1-netd] {s}");
}

/// The speaker's static key (Noise's X25519): made at the first start, kept in /data/zd1/net/key (the spec's key
/// file). Clients remember its public half with the speaker's address.
pub(crate) use net::KeyPair as Identity;

fn identity() -> Result<Identity, String> {
    let path = format!("{}/key", zd1_settings::NET);
    if let Some(i) = std::fs::read_to_string(&path).ok().as_deref().and_then(Identity::from_text) {
        return Ok(i);
    }
    let i = zd1_noise::generate();
    std::fs::create_dir_all(zd1_settings::NET).map_err(|e| format!("{}: {e}", zd1_settings::NET))?;
    zd1_sys::write_private(&path, &i.to_text()).map_err(|e| format!("{path}: {e}"))?;
    say(&format!("a new speaker key: {}", key_id(&i.public)));
    Ok(i)
}

/// `Pair`, across the sessions: one waits for the press at a time, and a refused one holds the next back 10 s.
#[derive(Default)]
pub(crate) struct PairGate {
    pub busy: bool,
    pub until: Option<Instant>,
}

/// What the sessions share: the pairing's gate, the provisional network settings (guard.rs).
#[derive(Default)]
pub(crate) struct Shared {
    pub pair: std::sync::Mutex<PairGate>,
    pub guard: std::sync::Mutex<guard::Guard>,
}

pub fn main() {
    let id = identity().unwrap_or_else(|e| zd1_sys::die("zd1-netd", &e));
    let l = TcpListener::bind(("0.0.0.0", net::PORT))
        .unwrap_or_else(|e| zd1_sys::die("zd1-netd", &format!("port {}: {e}", net::PORT)));
    say(&format!("on port {}, {}; speaker key {}", net::PORT, net::PATH, key_id(&id.public)));
    let (id, live, shared) = (Arc::new(id), Arc::new(AtomicUsize::new(0)), Arc::new(Shared::default()));
    let ticks = shared.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            ticks.guard.lock().unwrap().tick(Instant::now());
        }
    });
    for s in l.incoming() {
        let Ok(s) = s else { continue };
        if live.fetch_add(1, Ordering::SeqCst) >= net::SESSIONS {
            live.fetch_sub(1, Ordering::SeqCst);
            busy(s);
            continue;
        }
        let (id, live, shared) = (id.clone(), live.clone(), shared.clone());
        std::thread::spawn(move || {
            let peer = s.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            if let Err(e) = session::run(s, &id, &shared) {
                say(&format!("{peer}: {e}"));
            }
            live.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// More connections than sessions: an HTTP 503, closed.
fn busy(mut s: TcpStream) {
    use std::io::Write;
    let _ = s.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}
