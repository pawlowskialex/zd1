//! The page's WebSocket and `fetch` to the local network, which a web view refuses from an HTTPS page: the speaker's
//! link (`ws://SPEAKER/zd1`, docs/protocol/net.md; with the page's `Origin`, which the speaker checks) and BL2's web
//! updater (plain HTTP, its answers readable: a browser's `no-cors` hides them). Only local destinations: private and
//! link-local addresses, `.local` names, this computer. And what no browser gives a page: the speakers on the network
//! (mDNS) and this computer's networks (`window.zd1app`, polyfill.js).

use crate::{Page, Reply};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

/// What the page sends on a socket.
enum Out {
    Msg(Message),
    Close(u16, String),
}

pub struct Net {
    page: Page,
    socks: Mutex<HashMap<u64, mpsc::UnboundedSender<Out>>>,
    found: Arc<Mutex<HashMap<String, Value>>>,
}

/// What our speakers announce today: their AirPlay receiver (zd1-airplay), as model "Zeppelin Air". B&W's own
/// software announces AirPlay 1 only (`_raop._tcp`), so these are ours.
const AIRPLAY: &str = "_airplay._tcp.local.";

impl Net {
    pub fn new(page: Page) -> Net {
        let net = Net { page, socks: Mutex::default(), found: Arc::default() };
        net.browse();
        net
    }

    /// The speakers on the network, kept up to date for as long as the app runs (`speakers`).
    fn browse(&self) {
        let found = self.found.clone();
        let started = mdns_sd::ServiceDaemon::new().and_then(|d| d.browse(AIRPLAY).map(|rx| (d, rx)));
        let (daemon, rx) = match started {
            Ok(x) => x,
            Err(e) => return eprintln!("zd1-app: no mDNS: {e}"),
        };
        std::thread::spawn(move || {
            let _daemon = daemon; // browses as long as it lives
            while let Ok(ev) = rx.recv() {
                match ev {
                    mdns_sd::ServiceEvent::ServiceResolved(s) => {
                        if !s.get_property_val_str("model").is_some_and(|m| m.contains("Zeppelin")) {
                            continue;
                        }
                        let mut v4: Vec<_> = s.get_addresses_v4().into_iter().collect();
                        v4.sort_by_key(|a| (!a.is_private(), *a)); // the home network's before link-local
                        let Some(a) = v4.first() else { continue };
                        let name = s.get_fullname().strip_suffix(&format!(".{AIRPLAY}")).unwrap_or(s.get_fullname());
                        let host = s.get_hostname().trim_end_matches('.');
                        crate::log(&format!("mDNS: {name} at {a} ({host})"));
                        found.lock().unwrap().insert(
                            s.get_fullname().to_string(),
                            json!({ "name": name, "address": a.to_string(), "host": host }),
                        );
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, full) => {
                        found.lock().unwrap().remove(&full);
                    }
                    _ => {}
                }
            }
        });
    }

    /// A page (re)loading: the last one's sockets close.
    pub fn close_all(&self) {
        self.socks.lock().unwrap().clear();
    }

    pub fn request(self: &Arc<Self>, op: &str, m: Value, reply: Reply, rt: &Handle) {
        let url = m["url"].as_str().unwrap_or("").to_string();
        match op {
            "wsOpen" => {
                let Some(host) = local_url(&url, "ws://") else {
                    return reply.send(Err(("SecurityError", format!("{url} is not on the local network"))));
                };
                let sock = m["sock"].as_u64().unwrap_or(0);
                let origin = m["origin"].as_str().unwrap_or("").to_string();
                let (tx, rx) = mpsc::unbounded_channel();
                self.socks.lock().unwrap().insert(sock, tx);
                reply.send(Ok(Value::Null));
                let this = self.clone();
                rt.spawn(async move { this.socket(sock, url, host, origin, rx).await });
            }
            "wsSend" | "wsClose" => {
                let sock = m["sock"].as_u64().unwrap_or(0);
                let out = match (op, m["text"].as_str(), m["data"].as_str()) {
                    ("wsClose", ..) => {
                        Out::Close(m["code"].as_u64().unwrap_or(1000) as u16, m["reason"].as_str().unwrap_or("").into())
                    }
                    (_, Some(t), _) => Out::Msg(Message::text(t)),
                    (_, _, Some(d)) => match B64.decode(d) {
                        Ok(b) => Out::Msg(Message::binary(b)),
                        Err(e) => return reply.send(Err(("TypeError", e.to_string()))),
                    },
                    _ => return reply.send(Err(("TypeError", "nothing to send".into()))),
                };
                let sent = self.socks.lock().unwrap().get(&sock).is_some_and(|tx| tx.send(out).is_ok());
                reply.send(if sent || op == "wsClose" {
                    Ok(Value::Null)
                } else {
                    Err(("InvalidStateError", "the socket is closed".into()))
                });
            }
            "speakers" => {
                let mut l: Vec<Value> = self.found.lock().unwrap().values().cloned().collect();
                l.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                reply.send(Ok(Value::Array(l)));
            }
            "networks" => reply.send(Ok(Value::Array(networks()))),
            "fetch" => {
                if local_url(&url, "http://").is_none() {
                    return reply.send(Err(("SecurityError", format!("{url} is not on the local network"))));
                }
                rt.spawn(async move {
                    let r = tokio::task::spawn_blocking(move || fetch(&m)).await;
                    reply.send(match r {
                        Ok(Ok(v)) => Ok(v),
                        Ok(Err(e)) => Err(("TypeError", format!("Failed to fetch: {e}"))),
                        Err(e) => Err(("TypeError", e.to_string())),
                    });
                });
            }
            _ => reply.send(Err(("NotSupportedError", format!("no net.{op} here")))),
        }
    }

    /// One socket, to its end: what it receives to the page, what the page sends to it.
    async fn socket(&self, sock: u64, url: String, host: String, origin: String, mut rx: mpsc::UnboundedReceiver<Out>) {
        let ev = |kind: &str, mut v: Value| {
            v["sock"] = json!(sock);
            v["type"] = json!(kind);
            self.page.event("ws", v);
        };
        let connect = async {
            let mut req = url.as_str().into_client_request()?;
            if let Ok(o) = origin.parse() {
                req.headers_mut().insert("Origin", o);
            }
            tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(req))
                .await
                .map_err(|_| tokio_tungstenite::tungstenite::Error::Io(std::io::ErrorKind::TimedOut.into()))?
        };
        let ws = match connect.await {
            Ok((ws, _)) => ws,
            Err(e) => {
                crate::log(&format!("ws {host}: {e}"));
                ev("error", json!({}));
                ev("close", json!({ "code": 1006, "reason": "", "clean": false }));
                self.socks.lock().unwrap().remove(&sock);
                return;
            }
        };
        ev("open", json!({}));
        let (mut tx, mut inbox) = ws.split();
        let (mut code, mut reason, mut clean) = (1006u16, String::new(), false);
        loop {
            tokio::select! {
                m = inbox.next() => match m {
                    Some(Ok(Message::Binary(b))) => ev("message", json!({ "data": B64.encode(&b) })),
                    Some(Ok(Message::Text(t))) => ev("message", json!({ "text": t.as_str() })),
                    Some(Ok(Message::Close(f))) => {
                        if let Some(f) = f {
                            (code, reason) = (f.code.into(), f.reason.to_string());
                        } else {
                            code = 1005;
                        }
                        clean = true;
                        let _ = tx.close().await;
                        break;
                    }
                    Some(Ok(_)) => {} // ping, pong: tungstenite answers
                    Some(Err(e)) => {
                        crate::log(&format!("ws {host}: {e}"));
                        ev("error", json!({}));
                        break;
                    }
                    None => break,
                },
                o = rx.recv() => match o {
                    Some(Out::Msg(m)) => {
                        if let Err(e) = tx.send(m).await {
                            crate::log(&format!("ws {host}: {e}"));
                        }
                    }
                    Some(Out::Close(c, r)) => {
                        let _ = tx.send(Message::Close(Some(CloseFrame { code: c.into(), reason: r.into() }))).await;
                        // the speaker's close comes back on the inbox
                    }
                    None => break, // the page went
                },
            }
        }
        self.socks.lock().unwrap().remove(&sock);
        ev("close", json!({ "code": code, "reason": reason, "clean": clean }));
    }
}

/// One `fetch` (method, headers, body), redirects followed as a browser does: its status, headers and body.
fn fetch(m: &Value) -> Result<Value, ureq::Error> {
    // a host on the local network answers at once or isn't there (an address tried in a search for BL2)
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(3)))
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let mut req = ureq::http::Request::builder()
        .method(m["method"].as_str().unwrap_or("GET"))
        .uri(m["url"].as_str().unwrap_or(""));
    for h in m["headers"].as_array().into_iter().flatten() {
        if let (Some(k), Some(v)) = (h[0].as_str(), h[1].as_str()) {
            req = req.header(k, v);
        }
    }
    let body = match m["body"].as_str() {
        Some(b) => B64.decode(b).map_err(|e| ureq::Error::Io(std::io::Error::other(e)))?,
        None => Vec::new(),
    };
    let mut r = agent.run(req.body(body)?)?;
    let url = ureq::ResponseExt::get_uri(&r).to_string();
    let headers: Vec<Value> =
        r.headers().iter().filter_map(|(k, v)| Some(json!([k.as_str(), v.to_str().ok()?]))).collect();
    let status = r.status();
    let data = r.body_mut().with_config().limit(256 << 20).read_to_vec()?;
    Ok(json!({
        "status": status.as_u16(),
        "statusText": status.canonical_reason().unwrap_or(""),
        "headers": headers,
        "body": B64.encode(data),
        "url": url,
    }))
}

/// This computer's IPv4 networks on the local network (an address, its prefix length), for the page to look for what
/// doesn't announce itself (BL2).
fn networks() -> Vec<Value> {
    // not the virtual ones: VPN tunnels, VM and container bridges, Apple's peer-to-peer links
    const VIRTUAL: [&str; 12] =
        ["utun", "bridge", "vmenet", "vmnet", "vboxnet", "docker", "veth", "virbr", "awdl", "llw", "anpi", "tailscale"];
    let Ok(ifs) = if_addrs::get_if_addrs() else { return vec![] };
    ifs.into_iter()
        .filter(|i| !VIRTUAL.iter().any(|v| i.name.starts_with(v)))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if a.ip.is_private() || a.ip.is_link_local() => {
                Some(json!({ "interface": i.name, "address": a.ip.to_string(), "prefix": a.prefixlen }))
            }
            _ => None,
        })
        .collect()
}

/// The host of `url` if it starts with `scheme` and the host is on the local network.
fn local_url(url: &str, scheme: &str) -> Option<String> {
    let rest = url.get(..scheme.len()).filter(|s| s.eq_ignore_ascii_case(scheme)).map(|_| &url[scheme.len()..])?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    local_host(host).then(|| host.to_ascii_lowercase())
}

fn local_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    if h == "localhost" || (h.ends_with(".local") && h.len() > 6) {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => a.is_private() || a.is_link_local() || a.is_loopback(),
        Ok(IpAddr::V6(a)) => {
            let s = a.segments()[0];
            a.is_loopback() || s & 0xFE00 == 0xFC00 || s & 0xFFC0 == 0xFE80
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local() {
        assert_eq!(local_url("ws://192.168.50.20/zd1", "ws://").as_deref(), Some("192.168.50.20"));
        assert_eq!(local_url("ws://zeppelin.local/zd1", "ws://").as_deref(), Some("zeppelin.local"));
        assert_eq!(local_url("http://169.254.1.1:80/1000/x.asp", "http://").as_deref(), Some("169.254.1.1"));
        assert_eq!(local_url("ws://127.0.0.1:18080/zd1", "ws://").as_deref(), Some("127.0.0.1"));
        assert_eq!(local_url("ws://[fe80::1]/zd1", "ws://").as_deref(), Some("fe80::1"));
        assert!(local_url("ws://10.0.0.1@example.com/", "ws://").is_none());
        assert!(local_url("ws://example.com/zd1", "ws://").is_none());
        assert!(local_url("ws://8.8.8.8/zd1", "ws://").is_none());
        assert!(local_url("wss://192.168.50.20/zd1", "ws://").is_none());
        assert!(local_url("http://192.168.50.20.example.com/", "http://").is_none());
        assert!(local_url("http://.local/", "http://").is_none());
    }
}
