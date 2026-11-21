//! The speaker over the network (docs/protocol/net.md): the host link's messages to our module OS over a WebSocket
//! (`ws://SPEAKER/zd1`), inside a Noise_XX session. The front-end opens the WebSocket (a [`Socket`]: the browser's,
//! or a TCP one in `zd1`) and keeps this client's [`Identity`] and the speakers' keys it has met; [`NetLink::connect`]
//! does the handshake and is a [`Link`] from then on: a call is `u16 seq` + the request, what comes back are the
//! host link's records (REPLY, TEXT, EVENT), one per message. A client the speaker hasn't paired gets LOCKED for
//! anything but `Hello`: [`crate::Mcu`] then asks for `Pair` (a press of ON/OFF on the speaker), as it asks for
//! `Unlock` over USB.

use crate::link::Link;
use crate::spec::{msg, net, record};
use crate::text::key_id;
use crate::{Error, Result, codec};
use std::cell::RefCell;

/// The WebSocket under the session: binary messages both ways.
#[allow(async_fn_in_trait)]
pub trait Socket {
    async fn send(&self, data: &[u8]) -> Result<()>;
    /// The next message, waiting up to `ms` for it (0: only one that is there already); None: none came.
    async fn recv(&self, ms: u32) -> Result<Option<Vec<u8>>>;
    async fn sleep(&self, ms: u32);
}

/// A client's key (Noise's X25519), kept by the front-end in the spec's key file (`KeyPair::to_text`: the page in the
/// browser's storage, `zd1` in `~/.config/zd1`). The speaker pairs it.
pub use crate::spec::net::KeyPair as Identity;

/// A new key for this client: made once, then kept.
pub fn new_identity() -> Result<Identity> {
    let k = builder()?.generate_keypair().map_err(|e| Error::Failed(format!("a key: {e}")))?;
    Ok(Identity { private: k.private.try_into().unwrap(), public: k.public.try_into().unwrap() })
}

fn builder() -> Result<snow::Builder<'static>> {
    Ok(snow::Builder::new(net::NOISE.parse().map_err(|e| Error::Failed(format!("{}: {e}", net::NOISE)))?))
}

pub struct NetLink<S: Socket> {
    sock: S,
    noise: RefCell<snow::TransportState>,
    /// records that came while waiting for the hello's reply, for the next poll
    inbox: RefCell<Vec<u8>>,
    /// the speaker's key (the front-end remembers it with the address)
    pub speaker: [u8; 32],
    /// what this client calls itself when it pairs
    name: String,
}

impl<S: Socket> NetLink<S> {
    /// The handshake over `sock` as `me`. `known`: the key the speaker had at this address before; another one is an
    /// error (not the speaker this client paired with: forget it to pair again). `name`: what this client calls
    /// itself when it pairs.
    pub async fn connect(sock: S, me: &Identity, known: Option<[u8; 32]>, name: &str) -> Result<NetLink<S>> {
        let e = |e: snow::Error| Error::Failed(format!("the session's handshake: {e}"));
        let mut hs = builder()?
            .prologue(net::PROLOGUE)
            .and_then(|b| b.local_private_key(&me.private))
            .and_then(|b| b.build_initiator())
            .map_err(e)?;
        let mut buf = vec![0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(e)?;
        sock.send(&buf[..n]).await?;
        let m =
            sock.recv(10_000).await?.ok_or_else(|| Error::Timeout("the speaker didn't answer the handshake".into()))?;
        hs.read_message(&m, &mut buf).map_err(e)?;
        let speaker: [u8; 32] = hs
            .get_remote_static()
            .and_then(|k| k.try_into().ok())
            .ok_or_else(|| Error::Failed("no speaker key".into()))?;
        if known.is_some_and(|k| k != speaker) {
            return Err(Error::Device(format!(
                "the speaker at this address has another key ({}, not {}): not the one this client paired with \
                 (forget it to pair again)",
                key_id(&speaker),
                key_id(&known.unwrap())
            )));
        }
        let n = hs.write_message(&[], &mut buf).map_err(e)?;
        sock.send(&buf[..n]).await?;
        let noise = hs.into_transport_mode().map_err(e)?;
        Ok(NetLink {
            sock,
            noise: RefCell::new(noise),
            inbox: RefCell::default(),
            speaker,
            name: name.chars().take(32).collect(),
        })
    }

    async fn send(&self, plain: &[u8]) -> Result<()> {
        let mut out = vec![0u8; plain.len() + 16];
        let n = self.noise.borrow_mut().write_message(plain, &mut out).map_err(|e| Error::Failed(e.to_string()))?;
        self.sock.send(&out[..n]).await
    }

    /// The next record within `ms`, decrypted.
    async fn recv(&self, ms: u32) -> Result<Option<Vec<u8>>> {
        let Some(m) = self.sock.recv(ms).await? else { return Ok(None) };
        let mut out = vec![0u8; m.len()];
        let n = self
            .noise
            .borrow_mut()
            .read_message(&m, &mut out)
            .map_err(|e| Error::Failed(format!("a message from the speaker: {e}")))?;
        out.truncate(n);
        Ok(Some(out))
    }
}

impl<S: Socket> Link for NetLink<S> {
    /// The hello is a call like any other here (sequence number 0): the MCU's, through our OS, its request list cut to
    /// what the network link carries.
    async fn hello(&self) -> Result<Vec<u8>> {
        let mut call = 0u16.to_le_bytes().to_vec();
        call.extend(codec::encode_msg(&msg::Hello)?);
        self.send(&call).await?;
        let t0 = crate::now_ms();
        while crate::now_ms() - t0 < 10_000.0 {
            let Some(r) = self.recv(1000).await? else { continue };
            if let Some((record::REPLY, [0, 0, reply @ ..])) = record::split(&r).next() {
                return Ok(reply.to_vec());
            }
            self.inbox.borrow_mut().extend(r);
        }
        Err(Error::Timeout("the speaker didn't answer Hello".into()))
    }

    async fn call(&self, seq: u16, request: &[u8]) -> Result<bool> {
        let mut call = seq.to_le_bytes().to_vec();
        call.extend_from_slice(request);
        self.send(&call).await?;
        Ok(true)
    }

    async fn poll(&self) -> Result<Vec<u8>> {
        let mut out = std::mem::take(&mut *self.inbox.borrow_mut());
        while let Some(r) = self.recv(0).await? {
            out.extend(r);
        }
        Ok(out)
    }

    async fn sleep(&self, ms: u32) {
        self.sock.sleep(ms).await
    }

    fn unlock(&self) -> (&'static str, Vec<u8>) {
        ("Pair", codec::encode_msg(&msg::Pair { name: self.name.clone() }).expect("Pair encodes"))
    }

    fn network(&self) -> bool {
        true
    }
}

/// After a change over the network that can cut the client off (`spec::net::provisional`): a new session (`connect`:
/// the front-end's) once the change has taken effect (`SETTLE_S`), at the first address of `at` that answers (where
/// the speaker was, one the change set), until the speaker's time for it is nearly up (`CONFIRM_S`); the speaker keeps
/// the change once there is one, else it goes back by itself (docs/protocol/net.md). The address and the session.
pub async fn reach_again<L: Link>(
    at: &[String],
    ev: &dyn crate::Events,
    mut connect: impl AsyncFnMut(&str) -> Result<crate::Mcu<L>>,
) -> Result<(String, crate::Mcu<L>)> {
    let t0 = crate::now_ms();
    let secs = || ((crate::now_ms() - t0) / 1000.0) as u64;
    // a session opened sooner may still go over the old settings, and doesn't count
    while crate::now_ms() - t0 < net::SETTLE_S as f64 * 1000.0 {
        ev.wait("the change taking effect", secs());
        crate::sleep(1000).await;
    }
    while crate::now_ms() - t0 < (net::CONFIRM_S - 10) as f64 * 1000.0 {
        for a in at {
            if let Ok(m) = connect(a).await {
                ev.step(&format!("the speaker answers at {a}: the change is kept"));
                return Ok((a.clone(), m));
            }
        }
        ev.wait("the speaker again (the change is kept once it answers)", secs());
        crate::sleep(3000).await;
    }
    Err(Error::Timeout(format!(
        "the speaker didn't answer at {} after the change: it goes back to its settings from before",
        at.join(" or ")
    )))
}
