//! The network link's Noise session (docs/protocol/net.md): `Noise_XX_25519_ChaChaPoly_SHA256`
//! ([`zd1_hostlink::net::NOISE`]) with the link's prologue, on nivalis, whose builder rejects at compile time a key the
//! pattern doesn't take or lacks (proved in Lean: the fork's `lean/`). XX: `-> e; <- e, ee, s, es; -> s, se`. The
//! speaker answers ([`Speaker`]), a client starts ([`Client`]); each side's static key is a [`KeyPair`], and each learns
//! the other's: the client's after the third message is its identity to the speaker, the speaker's after the second
//! lets the client check it before it sends its own.

use getrandom::SysRng;
use nivalis::builder::NewBuilder;
use nivalis::crypto::cipher::chacha20::ChaChaPoly;
use nivalis::crypto::dh::{DH, DHKeypair, x25519::X25519dh};
use nivalis::crypto::hash::sha256::Sha256;
use nivalis::patterns::XX;
use nivalis::patterns::roles::RoleMarker;
pub use nivalis::patterns::roles::{Initiator, Responder};
use nivalis::state::handshake_state::{HandshakeResult, HandshakeState};
use nivalis::state::transport_state::TransportState;
use rand_core::UnwrapErr;
pub use zd1_hostlink::net::KeyPair;
use zd1_hostlink::net::PROLOGUE;

/// What a Noise step refused, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(e: impl core::fmt::Debug) -> Error {
    Error(format!("{e:?}"))
}

/// The system's randomness (the kernel's; the browser's in the page).
fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

/// A new static key: made once, then kept (the speaker in /data/zd1/net/key, a client in its key file).
pub fn generate() -> KeyPair {
    let k = X25519dh::generate_keypair(&mut rng());
    KeyPair { private: k.private().to_bytes(), public: *k.public().as_bytes() }
}

/// The handshake in progress, for the speaker's side or a client's.
pub struct Handshake<R: RoleMarker> {
    hs: HandshakeState<XX, R, X25519dh, ChaChaPoly, Sha256>,
}

/// The speaker's side: it reads the first and third messages, writes the second.
pub type Speaker = Handshake<Responder>;
/// A client's side: it writes the first and third messages, reads the second.
pub type Client = Handshake<Initiator>;

/// What a step of the handshake gives: more of it, or the session.
pub enum Step<R: RoleMarker> {
    Continue,
    Done(Transport<R>),
}

impl Speaker {
    /// The speaker's side of a new session, as `me`.
    pub fn new(me: &KeyPair) -> Result<Self, Error> {
        let hs = NewBuilder::<XX, Responder, X25519dh, ChaChaPoly, Sha256>::new()
            .local_static_key(X25519dh::privkey_from_bytes(&me.private).map_err(err)?)
            .prologue(PROLOGUE.to_vec())
            .build()
            .map_err(err)?;
        Ok(Handshake { hs })
    }
}

impl Client {
    /// A client's side of a new session, as `me`.
    pub fn new(me: &KeyPair) -> Result<Self, Error> {
        let hs = NewBuilder::<XX, Initiator, X25519dh, ChaChaPoly, Sha256>::new()
            .local_static_key(X25519dh::privkey_from_bytes(&me.private).map_err(err)?)
            .prologue(PROLOGUE.to_vec())
            .build()
            .map_err(err)?;
        Ok(Handshake { hs })
    }
}

impl<R: RoleMarker> Handshake<R> {
    /// The next handshake message to send (no payload), and the session if it was the last.
    pub fn write(&mut self) -> Result<(Vec<u8>, Step<R>), Error> {
        let mut buf = vec![0u8; 1024];
        match self.hs.write_message(&[], &mut buf, &mut rng()).map_err(err)? {
            HandshakeResult::Continue { bytes } => {
                buf.truncate(bytes);
                Ok((buf, Step::Continue))
            }
            HandshakeResult::Complete { bytes_written, transport_state, .. } => {
                buf.truncate(bytes_written);
                Ok((buf, Step::Done(Transport(transport_state))))
            }
        }
    }

    /// The peer's handshake message (no payload taken), and the session if it was the last.
    pub fn read(&mut self, message: &[u8]) -> Result<Step<R>, Error> {
        let mut payload = vec![0u8; message.len()];
        match self.hs.read_message(message, &mut payload).map_err(err)? {
            HandshakeResult::Continue { .. } => Ok(Step::Continue),
            HandshakeResult::Complete { transport_state, .. } => Ok(Step::Done(Transport(transport_state))),
        }
    }

    /// The peer's static key, once the handshake has it.
    pub fn remote(&self) -> Option<[u8; 32]> {
        self.hs.remote_static_pk().and_then(|k| k.try_into().ok())
    }
}

/// The session after the handshake: each message one Noise transport message.
pub struct Transport<R: RoleMarker>(TransportState<ChaChaPoly, X25519dh, R>);

/// The tag Noise adds to each message.
const TAG: usize = 16;

impl<R: RoleMarker> Transport<R> {
    /// The peer's static key (learnt in the handshake).
    pub fn remote(&self) -> Option<[u8; 32]> {
        self.0.remote_static_pk().and_then(|k| k.try_into().ok())
    }

    /// `plain`, encrypted, as one message.
    pub fn encrypt(&mut self, plain: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; plain.len() + TAG];
        let n = self.0.encrypt_message(&[], plain, &mut out).map_err(err)?;
        out.truncate(n);
        Ok(out)
    }

    /// A message from the peer, decrypted (Err: not one of this session's).
    pub fn decrypt(&mut self, message: &[u8]) -> Result<Vec<u8>, Error> {
        let len = message.len().checked_sub(TAG).ok_or_else(|| Error("a message shorter than its tag".into()))?;
        let mut out = vec![0u8; len];
        self.0.decrypt_message(&[], message, &mut out).map_err(err)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol name nivalis's types make is the one the spec names (the hash's first input: another name would
    /// make every handshake with the old clients fail).
    #[test]
    fn the_suite_is_the_specs() {
        use nivalis::crypto::cipher::Cipher;
        use nivalis::crypto::hash::Hash;
        use nivalis::patterns::Pattern;
        let name = format!("Noise_{}_{}_{}_{}", XX::NAME, X25519dh::NAME, ChaChaPoly::NAME, Sha256::NAME);
        assert_eq!(name, zd1_hostlink::net::NOISE);
    }

    /// A whole session: the handshake, each side learning the other's key (the client before it sends its own), then
    /// messages both ways; a tampered message is refused.
    #[test]
    fn a_session() {
        let (speaker_key, client_key) = (generate(), generate());
        let mut s = Speaker::new(&speaker_key).unwrap();
        let mut c = Client::new(&client_key).unwrap();
        let (m1, _) = c.write().unwrap();
        assert!(matches!(s.read(&m1).unwrap(), Step::Continue));
        let (m2, _) = s.write().unwrap();
        assert!(matches!(c.read(&m2).unwrap(), Step::Continue));
        assert_eq!(c.remote(), Some(speaker_key.public), "before the client's own key goes");
        let (m3, Step::Done(mut ct)) = c.write().unwrap() else { panic!("the client's session") };
        let Step::Done(mut st) = s.read(&m3).unwrap() else { panic!("the speaker's session") };
        assert_eq!(st.remote(), Some(client_key.public));
        for msg in [&b"a call"[..], b"", &[7u8; 40 * 1024]] {
            assert_eq!(st.decrypt(&ct.encrypt(msg).unwrap()).unwrap(), msg);
            assert_eq!(ct.decrypt(&st.encrypt(msg).unwrap()).unwrap(), msg);
        }
        let mut m = ct.encrypt(b"x").unwrap();
        m[0] ^= 1;
        assert!(st.decrypt(&m).is_err());
    }
}
