//! One client's connection (docs/protocol/net.md): the WebSocket's handshake (`GET /zd1`, anything else 404), Noise_XX
//! as the responder (three WebSocket messages), then calls (`u16 seq` + a request) answered in records (REPLY: `u16
//! seq` + the reply; EVENT: the State record whenever it changes), each one Noise transport message.

use crate::linkd::{Line, Linkd};
use crate::update::Update;
use crate::{Identity, PairGate, Shared, say};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};
use zd1_hostlink::codec::{self, Typed, error_reply};
use zd1_hostlink::{Err, MESSAGES, msg, net, record};

/// How long a handshake may take, and how often the session looks at zd1-linkd while the client is quiet.
const HANDSHAKE: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(200);
/// How often a quiet session looks whether its client has been forgotten (`net.forget`): its session then ends.
const RECHECK: Duration = Duration::from_secs(5);
/// A refused `Pair` holds the next one back this long.
const PAIR_BACKOFF: Duration = Duration::from_secs(10);

/// A connection, to its end: the handshakes, then the session. Err: it never got to a session.
pub fn run(s: TcpStream, id: &Identity, shared: &Shared) -> Result<(), String> {
    let peer = s.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    s.set_read_timeout(Some(HANDSHAKE)).map_err(|e| e.to_string())?;
    s.set_nodelay(true).map_err(|e| e.to_string())?;
    let cfg = WebSocketConfig::default()
        .read_buffer_size(8 * 1024)
        .write_buffer_size(0)
        .max_message_size(Some(net::MESSAGE_MAX))
        .max_frame_size(Some(net::MESSAGE_MAX));
    let path = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if req.uri().path() == net::PATH {
            Ok(resp)
        } else {
            let mut r = ErrorResponse::new(Some(format!("the speaker's link is ws://SPEAKER{}\n", net::PATH)));
            *r.status_mut() = tungstenite::http::StatusCode::NOT_FOUND;
            Err(r)
        }
    };
    let mut ws = tungstenite::accept_hdr_with_config(s, path, Some(cfg)).map_err(|e| format!("not our link: {e}"))?;

    // Noise_XX: -> e; <- e, ee, s, es; -> s, se. The client's static key is its identity from here on.
    let noise = |e: snow::Error| format!("the handshake: {e}");
    let mut hs = snow::Builder::new(net::NOISE.parse().map_err(noise)?)
        .prologue(net::PROLOGUE)
        .and_then(|b| b.local_private_key(&id.private))
        .and_then(|b| b.build_responder())
        .map_err(noise)?;
    let mut buf = vec![0u8; 65535];
    hs.read_message(&binary(&mut ws)?, &mut buf).map_err(noise)?;
    let n = hs.write_message(&[], &mut buf).map_err(noise)?;
    ws.send(Message::Binary(buf[..n].to_vec().into())).map_err(|e| e.to_string())?;
    hs.read_message(&binary(&mut ws)?, &mut buf).map_err(noise)?;
    let client: [u8; 32] = hs.get_remote_static().and_then(|k| k.try_into().ok()).ok_or("no client key")?;
    shared.guard.lock().unwrap().connected(&client, Instant::now()); // its network changes are kept
    let noise = hs.into_transport_mode().map_err(noise)?;
    ws.get_mut().set_read_timeout(Some(TICK)).map_err(|e| e.to_string())?;

    let mut s = Session {
        ws,
        noise,
        buf,
        client,
        who: format!("{peer} {}", zd1_text::key_id(&client)),
        paired: false,
        checked: Instant::now(),
        linkd: Linkd::open()?,
        update: Update::default(),
        last: Instant::now(),
    };
    s.check_paired();
    say(&format!("{}: {}", s.who, if s.paired { "paired" } else { "not paired: Hello and Pair only" }));
    let why = s.serve(shared).unwrap_or_else(|e| e);
    say(&format!("{}: {why}", s.who));
    Ok(())
}

/// The next binary message (the handshake's).
fn binary(ws: &mut WebSocket<TcpStream>) -> Result<Vec<u8>, String> {
    match ws.read().map_err(|e| format!("the handshake: {e}"))? {
        Message::Binary(b) => Ok(b.to_vec()),
        m => Err(format!("the handshake: a {} message", if m.is_text() { "text" } else { "control" })),
    }
}

struct Session {
    ws: WebSocket<TcpStream>,
    noise: snow::TransportState,
    buf: Vec<u8>,
    client: [u8; 32],
    who: String,
    paired: bool,
    /// when the controllers file was last read (at every request, and every `RECHECK` while the client is quiet)
    checked: Instant,
    linkd: Linkd,
    update: Update,
    last: Instant,
}

impl Session {
    /// Calls until the client closes, goes quiet for `net::IDLE_S`, or is forgotten: why it ended (Err: a failure).
    fn serve(&mut self, shared: &Shared) -> Result<String, String> {
        loop {
            match self.ws.read() {
                Ok(Message::Binary(b)) => {
                    self.last = Instant::now();
                    let n = self.noise.read_message(&b, &mut self.buf).map_err(|e| format!("a message: {e}"))?;
                    if n < 3 {
                        return Err("a call without a request".into());
                    }
                    let (seq, req) = (u16::from_le_bytes([self.buf[0], self.buf[1]]), self.buf[2..n].to_vec());
                    let reply = self.request(seq, &req, shared)?;
                    let mut r = seq.to_le_bytes().to_vec();
                    r.extend_from_slice(&reply);
                    self.send(record::REPLY, &r)?;
                    if let Some(slot) = self.update.installed.take() {
                        self.linkd.send(&format!("try {}", slot.name()))?;
                    }
                }
                Ok(Message::Close(_)) => return Ok("closed".into()),
                Ok(_) => {} // a ping: tungstenite answers it with the next write
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    let _ = self.ws.flush();
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok("closed".into());
                }
                Err(tungstenite::Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake)) => {
                    return Ok("gone (no close)".into());
                }
                Err(e) => return Err(e.to_string()),
            }
            while let Some(l) = self.linkd.next(Duration::from_millis(1))? {
                self.linkd_line(l)?;
            }
            if self.last.elapsed() > Duration::from_secs(net::IDLE_S as u64) {
                return Ok(format!("nothing for {} s", net::IDLE_S));
            }
            if self.paired && self.checked.elapsed() > RECHECK && !self.check_paired() {
                return Ok("forgotten (net.forget)".into());
            }
        }
    }

    /// A record to the client, as one Noise message.
    fn send(&mut self, kind: u8, payload: &[u8]) -> Result<(), String> {
        let r = record::encode(kind, payload);
        let mut out = vec![0u8; r.len() + 16];
        let n = self.noise.write_message(&r, &mut out).map_err(|e| e.to_string())?;
        out.truncate(n);
        self.ws.send(Message::Binary(out.into())).map_err(|e| e.to_string())
    }

    /// A line from zd1-linkd that isn't the answer awaited: the State record goes to a paired client.
    fn linkd_line(&mut self, l: Line) -> Result<(), String> {
        match l {
            Line::State(r) if self.paired => self.send(record::EVENT, &r),
            _ => Ok(()),
        }
    }

    /// Whether the client's key is paired: the controllers file read (a few hundred bytes). Not by its modification
    /// time: the module's clock starts at 1970 at every boot until NTP sets it, so a change can carry the time of
    /// the one before [EMU 2026-11-19: a pairing went unseen that way].
    fn check_paired(&mut self) -> bool {
        self.checked = Instant::now();
        self.paired = zd1_settings::controllers().iter().any(|c| c.key == self.client);
        self.paired
    }

    /// A request: the reply (an `Error` one for a refusal). Err: the session can't go on.
    fn request(&mut self, seq: u16, req: &[u8], shared: &Shared) -> Result<Vec<u8>, String> {
        let m = match codec::decode(req) {
            Ok(m) => m,
            Err(e) => return Ok(error_reply(Err::Malformed, &e.to_string())),
        };
        if !m.def.net {
            return Ok(error_reply(Err::Unknown, "not over the network (docs/protocol/net.md)"));
        }
        if !self.check_paired() && !matches!(m.name(), "Hello" | "Pair") {
            return Ok(error_reply(Err::Locked, "pair first: Pair, then ON/OFF on the speaker"));
        }
        match m.name() {
            "Pair" => self.pair(seq, &m.s("name"), &shared.pair),
            "SystemUpdate" => Ok(self.update.request(&m)),
            "SetSetting" if m.s("key") == "net" => {
                Ok(error_reply(Err::Arg, "the network link is switched over USB only"))
            }
            "SetSetting" if net::provisional(&m.s("key")) => {
                if let Err(e) = shared.guard.lock().unwrap().before(&self.client, &m.s("key"), Instant::now()) {
                    return Ok(error_reply(Err::Busy, &e));
                }
                self.call(seq, req)
            }
            "Hello" => {
                let r = self.call(seq, req)?;
                Ok(hello_here(&r).unwrap_or(r))
            }
            _ => self.call(seq, req),
        }
    }

    /// A request answered by zd1-linkd (the module's own, or the MCU's through it); State records meanwhile go on.
    fn call(&mut self, seq: u16, req: &[u8]) -> Result<Vec<u8>, String> {
        let wait = codec::decode(req).ok().map_or(2000, |m| m.def.timeout(&m.vals)).saturating_add(5000);
        self.linkd.send(&format!("host {seq} {}", zd1_sys::hex(req)))?;
        let end = Instant::now() + Duration::from_millis(wait as u64);
        while Instant::now() < end {
            match self.linkd.next(end.saturating_duration_since(Instant::now()))? {
                Some(Line::Reply(s, r)) if s == seq => return Ok(r),
                Some(l) => self.linkd_line(l)?,
                None => {}
            }
        }
        Ok(error_reply(Err::Module, "zd1-linkd didn't answer"))
    }

    /// `Pair`: a press of ON/OFF (the MCU's `Unlock`, which from the network always waits for it), then the key kept.
    fn pair(&mut self, seq: u16, name: &str, gate: &Mutex<PairGate>) -> Result<Vec<u8>, String> {
        let ok = || codec::encode_msg(&msg::Ok).expect("Ok encodes");
        if self.paired {
            return Ok(ok());
        }
        {
            let mut g = gate.lock().unwrap();
            if g.busy {
                return Ok(error_reply(Err::Busy, "another client waits for its press"));
            }
            if g.until.is_some_and(|t| Instant::now() < t) {
                return Ok(error_reply(Err::Busy, "a pairing was just refused: again in a few seconds"));
            }
            g.busy = true;
        }
        say(&format!("{}: \"{name}\" asks to pair: ON/OFF on the speaker", self.who));
        let r = self.call(seq, &codec::encode_msg(&msg::Unlock).expect("Unlock encodes"));
        let mut g = gate.lock().unwrap();
        g.busy = false;
        let r = r?;
        if codec::decode(&r).map(|m| m.name()) != Ok("Ok") {
            g.until = Some(Instant::now() + PAIR_BACKOFF);
            say(&format!("{}: not paired", self.who));
            return Ok(r);
        }
        let c = zd1_settings::Controller {
            key: self.client,
            name: name.to_string(),
            since: zd1_sys::utc_now().unwrap_or_default(),
        };
        if let Err(e) = zd1_settings::pair(&c) {
            return Ok(error_reply(Err::Arg, &e));
        }
        self.check_paired();
        say(&format!("{}: paired as \"{name}\"", self.who));
        Ok(ok())
    }
}

/// The MCU's `HelloReply` as this link has it: its requests cut to the ones that travel (`net`), with ours (the
/// module's: `Pair`, `SystemUpdate`).
fn hello_here(r: &[u8]) -> Option<Vec<u8>> {
    let m = codec::decode(r).ok()?;
    if m.name() != "HelloReply" {
        return None;
    }
    let mut h = msg::HelloReply::from_msg(&m);
    let mcu = std::mem::take(&mut h.requests);
    let has = |id: u8| mcu.get(id as usize / 8).is_some_and(|b| b >> (id % 8) & 1 == 1);
    let mut bits = vec![0u8; 32];
    for d in MESSAGES.iter().filter(|d| d.reply.is_some() && d.net && (d.module || has(d.id))) {
        bits[d.id as usize / 8] |= 1 << (d.id % 8);
    }
    h.requests = bits;
    codec::encode_msg(&h).ok()
}
