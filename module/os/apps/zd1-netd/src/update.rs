//! `SystemUpdate` (docs/protocol/net.md, System updates): a signed system image into the slot that doesn't run, as
//! zd1-update does it from the MCU's UART. BEGIN takes the signed header (`zd1-release`) and checks it before anything
//! is touched: a key we trust signed it, it is no older than what runs, nothing else updates, the running slot isn't on
//! trial. DATA hands the image in order to an installer thread (zd1-slot: erase, unpack as it comes); END waits for it
//! (the gzip stream's end, the image's length and SHA-256 against the header) and the slot is stamped; the session then
//! asks the MCU to start it once. A session that ends mid-way leaves the slot without a system, as ABORT does.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use zd1_hostlink::codec::{self, Msg, Val, error_reply};
use zd1_hostlink::{Err, UpdateOp, UpdateState};
use zd1_release::Header;
use zd1_slot::Slot;

/// Where the trusted keys are: the image's (the release key) and the data partition's (a developer's, put there over
/// USB).
const KEYS: &[&str] = &["/etc/zd1/keys", "/data/zd1/keys"];

/// A BEGIN whose header checked: what `start` begins.
pub struct Begin {
    header: Header,
    /// the key that signed it (its short name)
    key: String,
}

#[derive(Default)]
pub struct Update {
    run: Option<Running>,
    /// END went well: the slot to start once (the session tells zd1-linkd after the reply)
    pub installed: Option<Slot>,
}

struct Running {
    header: Header,
    slot: Slot,
    received: u64,
    tx: Option<SyncSender<Vec<u8>>>,
    worker: Option<JoinHandle<Result<String, String>>>,
    _one: std::fs::File,
}

impl Update {
    /// A `SystemUpdate` request: its `UpdateReply`.
    pub fn request(&mut self, m: &Msg) -> Vec<u8> {
        let op = UpdateOp::from(m.n("op") as u8);
        let r = match op {
            UpdateOp::Begin => return error_reply(Err::Arg, "BEGIN goes through `prepare` and `start`"),
            UpdateOp::Data => self.data(m.n("offset") as u64, m.bytes("data")),
            UpdateOp::End => self.end(),
            UpdateOp::Abort => {
                self.stop();
                Err("aborted".into())
            }
            _ => return error_reply(Err::Arg, "op: BEGIN, DATA, END or ABORT"),
        };
        match r {
            Ok(state) => self.reply(state, ""),
            Err(e) => {
                crate::say(&format!("update: {e}"));
                let r = self.reply(UpdateState::Failed, &e);
                self.stop();
                r
            }
        }
    }

    fn reply(&self, state: UpdateState, text: &str) -> Vec<u8> {
        let (sent, size, slot) = match &self.run {
            Some(r) => (r.received as u32, r.header.length as u32, slot_value(r.slot)),
            None => (0, 0, zd1_hostlink::Slot::None),
        };
        codec::encode(
            "UpdateReply",
            &[
                ("state", Val::N(state.value() as u32)),
                ("sent", Val::N(sent)),
                ("on_wire", Val::N(sent)),
                ("size", Val::N(size)),
                ("slot", Val::N(slot.value() as u32)),
                ("text", Val::from(zd1_text::cut(text, 96))),
            ],
        )
        .expect("UpdateReply encodes")
    }

    /// BEGIN, its first half: the header checked (signed by a key the speaker trusts, no older than what runs, not
    /// from a system on trial), nothing changed yet. Err: the `UpdateReply` that refuses it.
    pub fn prepare(&mut self, data: &[u8]) -> Result<Begin, Vec<u8>> {
        self.stop();
        self.check(data).map_err(|e| {
            crate::say(&format!("update: {e}"));
            self.reply(UpdateState::Failed, &e)
        })
    }

    /// BEGIN, its second half, once a person at the speaker allowed it (the session's `Allow`): the slot that doesn't
    /// run, written as the image comes. Its `UpdateReply`.
    pub fn start(&mut self, b: Begin) -> Vec<u8> {
        match self.begin(b) {
            Ok(state) => self.reply(state, ""),
            Err(e) => {
                crate::say(&format!("update: {e}"));
                let r = self.reply(UpdateState::Failed, &e);
                self.stop();
                r
            }
        }
    }

    fn check(&self, data: &[u8]) -> Result<Begin, String> {
        let (header, n) = Header::parse(data)?;
        if n != data.len() {
            return Err("BEGIN carries the header alone".into());
        }
        let keys = trusted_keys();
        let k = zd1_text::key_id(&keys[header.verify(&keys)?]);
        let running = zd1_sys::OsRelease::read("/etc/os-release").get("BUILD_DATE").to_string();
        if header.built < running {
            return Err(format!(
                "{} is older than the running system ({running}): going back is USB's",
                header.version
            ));
        }
        let b = zd1_slot::boot();
        if b.running.is_some() && b.running != b.good {
            return Err("the running system is on trial: its ready first (the good slot is the way back)".into());
        }
        Ok(Begin { header, key: k })
    }

    fn begin(&mut self, Begin { header, key: k }: Begin) -> Result<UpdateState, String> {
        let b = zd1_slot::boot();
        let one = zd1_slot::update_lock()?;
        let slot = b.target();
        let (tx, rx) = sync_channel::<Vec<u8>>(4);
        let (stamp, h) = (header.stamp(), header.clone());
        crate::say(&format!("update: {stamp} (key {}) into slot {}", k, slot.name()));
        let worker = std::thread::spawn(move || {
            let mut image = Image { rx, chunk: Vec::new(), at: 0, hash: Sha256::new(), bytes: 0 };
            zd1_slot::install(slot, &stamp, &mut image, &mut |image: &mut Image| {
                std::io::copy(image, &mut std::io::sink()).map_err(|e| format!("the image: {e}"))?; // to END
                if image.bytes != h.length {
                    return Err(format!("{} bytes, the header says {}", image.bytes, h.length));
                }
                let got: [u8; 32] = std::mem::take(&mut image.hash).finalize().into();
                if got != h.sha256 {
                    return Err("its SHA-256 isn't the signed one".into());
                }
                Ok(())
            })
        });
        self.run = Some(Running { header, slot, received: 0, tx: Some(tx), worker: Some(worker), _one: one });
        Ok(UpdateState::Streaming)
    }

    fn data(&mut self, offset: u64, data: &[u8]) -> Result<UpdateState, String> {
        let r = self.run.as_mut().ok_or("no update: BEGIN first")?;
        if offset != r.received {
            return Err(format!("DATA at {offset}, {} expected", r.received));
        }
        if r.received + data.len() as u64 > r.header.length {
            return Err("more than the header's length".into());
        }
        let sent = r.tx.as_ref().is_some_and(|tx| tx.send(data.to_vec()).is_ok());
        if !sent {
            // the installer stopped: why
            r.tx = None;
            let why = r.worker.take().map(|w| w.join()).map(|j| j.unwrap_or_else(|_| Err("it panicked".into())));
            return Err(match why {
                Some(Err(e)) => e,
                _ => "the installer stopped".into(),
            });
        }
        r.received += data.len() as u64;
        Ok(UpdateState::Streaming)
    }

    fn end(&mut self) -> Result<UpdateState, String> {
        let r = self.run.as_mut().ok_or("no update: BEGIN first")?;
        if r.received != r.header.length {
            return Err(format!("END after {} of {} bytes", r.received, r.header.length));
        }
        r.tx = None; // the image's end
        let done = r.worker.take().map(|w| w.join().unwrap_or_else(|_| Err("it panicked".into())));
        match done {
            Some(Ok(m)) => {
                crate::say(&format!("update: {m}; starting it once"));
                self.installed = Some(r.slot);
                Ok(UpdateState::Done)
            }
            Some(Err(e)) => Err(e),
            None => Err("the installer stopped".into()),
        }
    }

    /// The update in hand dropped: its installer sees the image end early and leaves the slot without a system.
    fn stop(&mut self) {
        if let Some(mut r) = self.run.take() {
            r.tx = None;
            if let Some(w) = r.worker.take() {
                let _ = w.join();
            }
        }
    }
}

impl Drop for Update {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The image as DATA brings it, hashed and counted.
struct Image {
    rx: Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
    hash: Sha256,
    bytes: u64,
}

impl Read for Image {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.chunk.len() {
            match self.rx.recv() {
                Ok(c) => (self.chunk, self.at) = (c, 0),
                Err(_) => return Ok(0), // END (or the session gone)
            }
        }
        let n = buf.len().min(self.chunk.len() - self.at);
        buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        self.hash.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}

/// The keys an update may be signed with (`*.pub`: one 64-hex-digit key each).
fn trusted_keys() -> Vec<[u8; 32]> {
    KEYS.iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
        .filter_map(|e| zd1_release::read_key(&std::fs::read_to_string(e.path()).ok()?))
        .collect()
}

fn slot_value(s: Slot) -> zd1_hostlink::Slot {
    match s {
        Slot::A => zd1_hostlink::Slot::A,
        Slot::B => zd1_hostlink::Slot::B,
    }
}
