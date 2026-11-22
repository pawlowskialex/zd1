//! `--net HOST`: the speaker over the network (docs/protocol/net.md). A WebSocket over TCP to `ws://HOST/zd1`, this
//! computer's key, and the speakers it has met (their keys by the address they were reached at), in
//! `~/.config/zd1` (`$XDG_CONFIG_HOME/zd1`): `net-key` (PRIVATE PUBLIC, hex; only this user reads it) and `speakers`
//! (ADDRESS KEY a line).

use std::cell::RefCell;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use tungstenite::{Message, WebSocket};
use zd1_host::net::{Identity, NetLink, Socket, new_identity};
use zd1_host::spec::net;
use zd1_host::text::{key_id, unhex_n};
use zd1_host::{Error, Events, Mcu, Result};

/// The WebSocket, blocking with a read timeout per receive.
pub struct TcpWs(RefCell<WebSocket<TcpStream>>);

impl Socket for TcpWs {
    async fn send(&self, data: &[u8]) -> Result<()> {
        self.0.borrow_mut().send(Message::Binary(data.to_vec().into())).map_err(gone)
    }

    async fn recv(&self, ms: u32) -> Result<Option<Vec<u8>>> {
        let mut ws = self.0.borrow_mut();
        ws.get_mut().set_read_timeout(Some(Duration::from_millis(ms.max(1) as u64))).map_err(|e| gone(e.into()))?;
        match ws.read() {
            Ok(Message::Binary(b)) => Ok(Some(b.to_vec())),
            Ok(Message::Close(_)) => Err(Error::Gone("the speaker closed the connection".into())),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Ok(None)
            }
            Err(e) => Err(gone(e)),
        }
    }

    async fn sleep(&self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}

/// A session ends with the WebSocket's close.
impl Drop for TcpWs {
    fn drop(&mut self) {
        let mut ws = self.0.borrow_mut();
        if ws.close(None).is_ok() {
            let _ = ws.flush();
        }
    }
}

fn gone(e: tungstenite::Error) -> Error {
    Error::Device(format!("the connection to the speaker: {e}"))
}

fn dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| Error::Failed("no HOME for ~/.config/zd1".into()))?;
    let d = base.join("zd1");
    std::fs::create_dir_all(&d).map_err(|e| Error::Failed(format!("{}: {e}", d.display())))?;
    Ok(d)
}

/// This computer's key: made the first time.
fn identity() -> Result<Identity> {
    let p = dir()?.join("net-key");
    if let Some(i) = std::fs::read_to_string(&p).ok().as_deref().and_then(Identity::from_text) {
        return Ok(i);
    }
    let i = new_identity()?;
    crate::files::write_private(&p, i.to_text().as_bytes())?;
    Ok(i)
}

/// The key the speaker at `addr` had before.
fn known(addr: &str) -> Result<Option<[u8; 32]>> {
    let t = std::fs::read_to_string(dir()?.join("speakers")).unwrap_or_default();
    Ok(t.lines().find_map(|l| l.split_once(' ').filter(|(a, _)| *a == addr).and_then(|(_, k)| unhex_n::<32>(k))))
}

fn remember(addr: &str, k: &[u8; 32]) -> Result<()> {
    let p = dir()?.join("speakers");
    let mut t = std::fs::read_to_string(&p).unwrap_or_default();
    t += &format!("{addr} {}\n", zd1_host::hex(k));
    std::fs::write(&p, t).map_err(|e| Error::Failed(format!("{}: {e}", p.display())))
}

/// What this computer calls itself when it pairs.
fn client_name() -> String {
    let mut b = [0u8; 64];
    let host = unsafe { libc::gethostname(b.as_mut_ptr() as *mut libc::c_char, b.len()) } == 0;
    let n = b.iter().position(|&c| c == 0).unwrap_or(0);
    if host && n > 0 { format!("zd1 on {}", String::from_utf8_lossy(&b[..n])) } else { "zd1".into() }
}

/// The speaker at `addr` (HOST or HOST:PORT): the WebSocket, the session (the speaker's key checked against the one
/// met at this address before, kept the first time), the hello.
pub async fn connect(addr: &str, ev: Rc<dyn Events>) -> Result<Mcu<NetLink<TcpWs>>> {
    let hostport = if addr.contains(':') { addr.to_string() } else { format!("{addr}:{}", net::PORT) };
    let to = hostport
        .to_socket_addrs()
        .map_err(|e| Error::Device(format!("{addr}: {e}")))?
        .next()
        .ok_or_else(|| Error::Device(format!("{addr}: no address")))?;
    let tcp = TcpStream::connect_timeout(&to, Duration::from_secs(5))
        .map_err(|e| Error::Device(format!("{addr}: {e} (the speaker on the network, our OS running?)")))?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let cfg = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(net::MESSAGE_MAX))
        .max_frame_size(Some(net::MESSAGE_MAX));
    let (ws, _) = tungstenite::client::client_with_config(format!("ws://{hostport}{}", net::PATH), tcp, Some(cfg))
        .map_err(|e| Error::Device(format!("{addr}: not our speaker's link: {e}")))?;
    let pinned = known(addr)?;
    let link =
        NetLink::connect(TcpWs(RefCell::new(ws)), &identity()?, pinned, &client_name()).await.map_err(|e| match e {
            Error::Device(s) => Error::Device(format!(
                "{s}: its line in {}",
                dir().map(|d| d.join("speakers").display().to_string()).unwrap_or_default()
            )),
            e => e,
        })?;
    if pinned.is_none() {
        remember(addr, &link.speaker)?;
        ev.step(&format!("a new speaker at {addr}: its key {}", key_id(&link.speaker)));
    }
    Mcu::new(link, ev, false).await
}
