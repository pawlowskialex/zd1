//! The web page's side: host/core's flows over WebUSB (nusb's backend), for the page in www/. No flow lives here;
//! this turns the core's types into JavaScript's and back.
//!
//! The page asks the browser for the speaker (navigator.usb.requestDevice: a click), then `Speaker.connect(events)`
//! finds it among the devices it may use. `events` is the page's object: text(s) (the MCU's log), prompt(how, s) (the
//! speaker waits for a person at it: "press" ON/OFF, "pair" or "allow": ON/OFF held 5 s), step(s), phase(n, of, what), progress(what, done, total), cancelled() -> bool, wantsDevice(vid, pid, what) (a
//! device to grant in the browser's chooser: the DFU bootloader during a firmware update), backup(tar, name, holds)
//! (keep the MCU's backup, host/core's `backup`: awaited before anything is written; a rejection stops the flash).
//!
//! A speaker that is in its bootloader (DFU) instead: `Bootloader.connect(events)`, the same `events`.
//!
//! Over the network (docs/protocol/net.md): `Speaker.connectNet(address, identity, known, events)`, this browser's key
//! from `newIdentity` and the speaker's from before (`speakerKey`), both kept by the page; the same `Speaker` then,
//! with what goes over USB only refused (the boot image, the MCU's firmware, the console).

use js_sys::{Array, Object, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use zd1_host::backup::Backup;
use zd1_host::codec::{self, Msg, Typed, Val};
use zd1_host::image::{self, Kind, McuImage};
use zd1_host::net::{Identity, NetLink};
use zd1_host::spec::net as net_spec;
use zd1_host::spec::{self, Ty};
use zd1_host::usb::{Bus, Iface, NusbBus, NusbDev, UsbError};
use zd1_host::{Error, Events, Mcu, dfu, flash, install, msg, settings, speaker, update};

mod net;
use net::WebSock;

#[wasm_bindgen]
extern "C" {
    /// The page's side of the flows (module doc).
    pub type JsEvents;
    #[wasm_bindgen(method)]
    fn text(this: &JsEvents, s: &str);
    #[wasm_bindgen(method)]
    fn prompt(this: &JsEvents, how: &str, s: &str);
    #[wasm_bindgen(method)]
    fn step(this: &JsEvents, s: &str);
    #[wasm_bindgen(method)]
    fn phase(this: &JsEvents, n: u32, of: u32, what: &str);
    #[wasm_bindgen(method)]
    fn progress(this: &JsEvents, what: &str, done: f64, total: f64);
    #[wasm_bindgen(method)]
    fn wait(this: &JsEvents, what: &str, secs: f64);
    #[wasm_bindgen(method)]
    fn cancelled(this: &JsEvents) -> bool;
    #[wasm_bindgen(method, js_name = wantsDevice)]
    fn wants_device(this: &JsEvents, vid: u16, pid: u16, what: &str);
    #[wasm_bindgen(method)]
    fn backup(this: &JsEvents, tar: Uint8Array, name: &str, holds: &str) -> JsValue;
    /// The module's console's bytes, as they come (`Speaker.consoleOpen`).
    #[wasm_bindgen(method)]
    fn console(this: &JsEvents, b: Uint8Array);
}

/// The backup to events.backup, awaited: its rejection is the error.
async fn keep(js: &JsEvents, b: &Backup) -> zd1_host::Result<()> {
    let r = js.backup(Uint8Array::from(b.tar().as_slice()), &b.file_name(), &b.holds());
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&r)).await.map(drop).map_err(|e| {
        let why = Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string()).or_else(|| e.as_string());
        Error::Failed(format!("the backup wasn't kept: {}", why.unwrap_or_default()))
    })
}

struct Ev(JsEvents);

impl Events for Ev {
    fn text(&self, b: &[u8]) {
        self.0.text(&String::from_utf8_lossy(b))
    }
    fn prompt(&self, how: zd1_host::Confirm, s: &str) {
        self.0.prompt(how.as_str(), s)
    }
    fn step(&self, s: &str) {
        self.0.step(s)
    }
    fn phase(&self, n: usize, of: usize, what: &str) {
        self.0.phase(n as u32, of as u32, what)
    }
    fn progress(&self, what: &str, done: u64, total: u64) {
        self.0.progress(what, done as f64, total as f64)
    }
    fn wait(&self, what: &str, secs: u64) {
        self.0.wait(what, secs as f64)
    }
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
    fn wants_device(&self, vid: u16, pid: u16, what: &str) {
        self.0.wants_device(vid, pid, what)
    }
    fn patient(&self) -> bool {
        true
    }
}

/// The WebUSB backend's warnings: nusb logs the browser's own reason ("Unable to open device", ...) and returns
/// "WebUSB error (see logs)". They go to the console, and the last one into the error the page shows.
struct ConsoleLog;
thread_local! {
    static LAST_WARNING: RefCell<String> = const { RefCell::new(String::new()) };
}
impl log::Log for ConsoleLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let s = r.args().to_string();
            console_warn(&s);
            LAST_WARNING.with(|l| *l.borrow_mut() = s);
        }
    }
    fn flush(&self) {}
}
static LOG: ConsoleLog = ConsoleLog;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(s: &str);
}

#[wasm_bindgen(start)]
fn start_log() {
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// An error for JavaScript: its message, and `code` (host/core's `Error::code`: GONE, TIMEOUT, LOCKED, DENIED,
/// CANCELLED, ARG, ...).
fn js_err(e: Error) -> JsValue {
    let mut msg = e.to_string();
    if msg.contains("(see logs)") {
        let why = LAST_WARNING.with(|l| l.borrow().clone());
        if !why.is_empty() {
            msg = msg.replace("(see logs)", &format!("({why})"));
        }
    }
    let err = js_sys::Error::new(&msg);
    let _ = Reflect::set(&err, &"code".into(), &e.code().into());
    err.into()
}

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &k.into(), &v.into());
}

/// How a system went into a slot: { outcome: "running" | "fellBack", slot, now } (`now`: the slot the module runs).
fn outcome(o: &update::Outcome) -> Object {
    let (outcome, slot, now) = match *o {
        update::Outcome::FellBack { slot, now } => ("fellBack", slot, now),
        update::Outcome::Running(s) | update::Outcome::Installed(s) => ("running", s, s),
    };
    let r = Object::new();
    set(&r, "outcome", outcome);
    set(&r, "slot", slot.to_string());
    set(&r, "now", now.to_string());
    r
}

/// A message as a JavaScript object: its fields, and `msg` its name. Enums as their values (the page has the names:
/// `spec()`).
fn to_js(m: &Msg) -> JsValue {
    let o = Object::new();
    set(&o, "msg", m.name());
    for (f, v) in m.fields() {
        let v: JsValue = match (f.ty, v) {
            (Ty::Str(_), Val::S(s)) => String::from_utf8_lossy(s).as_ref().into(),
            (_, Val::S(s)) => Uint8Array::from(s.as_slice()).into(),
            (_, Val::B(b)) => (*b).into(),
            (_, Val::N(n)) => (*n).into(),
        };
        set(&o, f.name, v);
    }
    o.into()
}

/// A request's fields from a JavaScript object, by the spec's types (an enum also by its name). A mask left out
/// (`Prefs.change`) says the fields given.
fn from_js(name: &str, o: &JsValue) -> Result<Vec<u8>, JsValue> {
    let m = spec::message(name).ok_or_else(|| JsValue::from(js_sys::Error::new(&format!("no message {name}"))))?;
    let given = |f: &str| !Reflect::get(o, &f.into()).unwrap_or(JsValue::UNDEFINED).is_undefined();
    let mut f = Vec::new();
    for field in m.fields {
        let v = Reflect::get(o, &field.name.into()).unwrap_or(JsValue::UNDEFINED);
        if v.is_undefined() {
            if field.mask {
                let bits = m.fields.iter().filter(|g| given(g.name)).filter_map(|g| m.mask_bit(g.name)).sum::<u8>();
                f.push((field.name, Val::N(bits as u32)));
            }
            continue;
        }
        let bad = || JsValue::from(js_sys::Error::new(&format!("{name}.{}: {v:?}", field.name)));
        f.push((
            field.name,
            match field.ty {
                Ty::Bool => Val::B(v.as_bool().ok_or_else(bad)?),
                Ty::Str(_) => Val::from(v.as_string().ok_or_else(bad)?),
                Ty::Bytes(_) => Val::S(Uint8Array::new(&v).to_vec()),
                Ty::Enum(e) if v.is_string() => {
                    Val::N(codec::enum_value(e, &v.as_string().unwrap()).ok_or_else(bad)? as u32)
                }
                _ => Val::N(v.as_f64().ok_or_else(bad)? as u32),
            },
        ));
    }
    codec::encode(name, &f).map_err(|e| js_sys::Error::new(&e.to_string()).into())
}

/// The protocol's enums, for the page (names in upper case, as the reference has them): { Source: { NONE: 0, USB: 1 }, ... },
/// `settings`: the State record's writable fields and `actions` the module's actions ({ key, name, kind, max, format,
/// default, doc } each), `proto` the host link's version.
#[wasm_bindgen]
pub fn spec() -> JsValue {
    let o = Object::new();
    for e in spec::ENUMS {
        let v = Object::new();
        for (n, x, _) in e.values {
            set(&v, &n.to_uppercase(), *x);
        }
        set(&o, e.name, v);
    }
    let fields = |it: &mut dyn Iterator<Item = &'static spec::Field>| {
        let a = Array::new();
        for f in it {
            let o = Object::new();
            set(&o, "key", f.key());
            set(&o, "name", f.name);
            let (kind, max): (&str, u32) = match f.ty {
                Ty::Str(m) => ("text", m as u32),
                Ty::Bool => ("bool", 0),
                Ty::Enum(e) => (e, 0),
                Ty::U8 => ("number", 255),
                Ty::U16 => ("number", 65535),
                Ty::U32 => ("number", u32::MAX),
                Ty::Bytes(m) => ("bytes", m as u32),
            };
            set(&o, "kind", kind);
            set(&o, "max", max);
            set(&o, "format", f.format);
            set(&o, "default", f.default);
            set(&o, "doc", f.doc);
            a.push(&o);
        }
        a
    };
    set(&o, "settings", fields(&mut spec::settings()));
    set(&o, "actions", fields(&mut spec::actions()));
    set(&o, "proto", spec::VERSION);
    let usb = Object::new();
    set(&usb, "vid", spec::usb::VID);
    set(&usb, "pid", spec::usb::PID);
    set(&usb, "dfuPid", dfu::PID);
    set(&o, "usb", usb);
    o.into()
}

/// What a file is (host/core's `image::identify`): { kind: "mcu", version, built, start, end } (Intel HEX or the app
/// region's .bin), { kind: "backup", version, when } (an MCU backup: installs as MCU firmware), { kind: "system",
/// version, stamp, size } (a gzip'd cpio of a slot's root), { kind: "signed", version, built, stamp, size } (one signed
/// for the network update), { kind: "fw", ours, boot, version, built, size } (a B&W .fw: the Lightning model's first
/// install through its BL2) or { kind: "bcd", build, ours, version } (the module's boot image: ours or the stock one;
/// version: ours's identity, `zd1.boot=`).
#[wasm_bindgen(js_name = imageInfo)]
pub fn image_info(name: &str, data: &[u8]) -> Result<JsValue, JsValue> {
    let o = Object::new();
    match image::identify(name, data).map_err(js_err)? {
        Kind::System { version, stamp } => {
            set(&o, "kind", "system");
            set(&o, "version", version);
            set(&o, "stamp", stamp);
            set(&o, "size", data.len() as f64);
        }
        Kind::Signed { version, built, stamp } => {
            set(&o, "kind", "signed");
            set(&o, "version", version);
            set(&o, "built", built);
            set(&o, "stamp", stamp);
            set(&o, "size", data.len() as f64);
        }
        Kind::Fw { bcd_ours, boot, mcu } => {
            set(&o, "kind", "fw");
            set(&o, "ours", bcd_ours && mcu.is_some());
            set(&o, "boot", boot);
            if let Some(i) = mcu {
                set(&o, "version", i.version);
                set(&o, "built", i.built);
            }
            set(&o, "size", data.len() as f64);
        }
        Kind::Bcd { build, ours, boot } => {
            set(&o, "kind", "bcd");
            set(&o, "build", build);
            set(&o, "ours", ours);
            if !boot.is_empty() {
                set(&o, "version", boot);
            }
        }
        Kind::Backup(b) => {
            set(&o, "kind", "backup");
            set(&o, "version", b.holds());
            set(&o, "when", b.when);
        }
        Kind::Mcu(img) => {
            let (s, e) = img.app_span().map_err(js_err)?;
            set(&o, "kind", "mcu");
            if let Some(i) = img.ident() {
                set(&o, "version", i.version);
                set(&o, "built", i.built);
            }
            set(&o, "start", s);
            set(&o, "end", e);
        }
    }
    Ok(o.into())
}

/// A message of the spec from a JavaScript object (as `call` takes fields, or as a reply came).
fn typed_from_js<T: Typed>(o: &JsValue) -> Result<T, JsValue> {
    let b = from_js(T::NAME, o)?;
    Ok(T::from_msg(&codec::decode(&b).map_err(|e| js_err(e.into()))?))
}

fn sentence((text, tone): (String, speaker::Tone)) -> JsValue {
    let o = Object::new();
    set(&o, "text", text);
    set(&o, "tone", tone.name());
    o.into()
}

/// The Wi-Fi as a sentence (host/core's `speaker::wifi`): { text, tone } ("", "ok" or "warn"), from the State record
/// and the known networks.
#[wasm_bindgen(js_name = wifiSentence)]
pub fn wifi_sentence(state: JsValue, known: Array) -> Result<JsValue, JsValue> {
    let st: msg::State = typed_from_js(&state)?;
    let known = known.iter().map(|k| typed_from_js::<msg::KnownNetwork>(&k)).collect::<Result<Vec<_>, _>>()?;
    Ok(sentence(speaker::wifi(&st, &known)))
}

/// The amp in words, from an InfoReply ("on", "locked until the speaker restarts", ...).
#[wasm_bindgen(js_name = ampSentence)]
pub fn amp_sentence(info: JsValue) -> Result<String, JsValue> {
    Ok(speaker::amp(&typed_from_js(&info)?))
}

/// The gate in words, from an InfoReply.
#[wasm_bindgen(js_name = gateSentence)]
pub fn gate_sentence(info: JsValue) -> Result<String, JsValue> {
    Ok(speaker::gate(&typed_from_js(&info)?))
}

/// The speaker over one of its links: WebUSB, or the network (our module OS, docs/protocol/net.md).
enum Conn {
    Usb(Mcu<NusbDev>),
    Net(Mcu<NetLink<WebSock>>),
}

/// `$body` with `$m` the speaker's `Mcu`, whichever its link.
macro_rules! on {
    ($c:expr, $m:ident => $body:expr) => {
        match $c {
            Conn::Usb($m) => $body,
            Conn::Net($m) => $body,
        }
    };
}

/// Where a network connection goes, to open it again (after an update the speaker restarts).
#[derive(Clone)]
struct NetTo {
    addr: String,
    me: Identity,
    speaker: [u8; 32],
}

impl NetTo {
    async fn open(&self, ev: Rc<dyn Events>) -> zd1_host::Result<Mcu<NetLink<WebSock>>> {
        let link = NetLink::connect(WebSock::open(&self.addr).await?, &self.me, Some(self.speaker), CLIENT).await?;
        Mcu::new(link, ev, false).await
    }
}

/// What this page calls itself when it pairs (the speaker's `Controllers` record).
const CLIENT: &str = "the web page";

/// Our MCU firmware, over WebUSB or the network.
#[wasm_bindgen]
pub struct Speaker {
    m: Rc<RefCell<Option<Conn>>>,
    closed: Cell<bool>,
    ev: Rc<dyn Events>,
    js: JsEvents,
    /// over the network: where, to connect again (the address may change: `reachAgain`)
    net: RefCell<Option<NetTo>>,
}

/// A setting or action that, changed over the network, can cut the page off the speaker: kept once the page connects
/// again (`Speaker.reachAgain`), else the speaker goes back (docs/protocol/net.md).
#[wasm_bindgen]
pub fn provisional(key: &str) -> bool {
    net_spec::provisional(key)
}

/// A new key for this browser to pair with speakers over the network: the text to keep (the page keeps it in its
/// storage; `Speaker.connectNet` takes it).
#[wasm_bindgen(js_name = newIdentity)]
pub fn new_identity() -> Result<String, JsValue> {
    zd1_host::net::new_identity().map(|i| i.to_text()).map_err(js_err)
}

#[wasm_bindgen]
impl Speaker {
    /// The speaker among the devices the page may use (undefined: none; ask with navigator.usb.requestDevice).
    pub async fn connect(events: JsEvents) -> Result<Option<Speaker>, JsValue> {
        let ev: Rc<dyn Events> = Rc::new(Ev(events.clone().unchecked_into()));
        match Mcu::open(&NusbBus, ev.clone(), true).await.map_err(js_err)? {
            Some(m) => Ok(Some(Speaker::with(Conn::Usb(m), ev, events, None))),
            None => Ok(None),
        }
    }

    /// The speaker on the network at `addr` (an address, NAME.local; HOST:PORT), this browser being `identity`
    /// (`newIdentity`'s text). `known`: the speaker's key met at this address before (hex; another one there is an
    /// error: not the speaker this browser paired with), else the page keeps `speakerKey` now. The first request that
    /// needs it asks for the pairing (events.prompt: ON/OFF held 5 s on the speaker).
    #[wasm_bindgen(js_name = connectNet)]
    pub async fn connect_net(
        addr: String,
        identity: String,
        known: Option<String>,
        events: JsEvents,
    ) -> Result<Speaker, JsValue> {
        let ev: Rc<dyn Events> = Rc::new(Ev(events.clone().unchecked_into()));
        let me =
            Identity::from_text(&identity).ok_or_else(|| js_err(Error::Failed("not a key (newIdentity)".into())))?;
        let known = known
            .as_deref()
            .map(|k| zd1_host::text::unhex_n::<32>(k).ok_or_else(|| js_err(Error::Failed("not a speaker key".into()))))
            .transpose()?;
        let sock = WebSock::open(&addr).await.map_err(js_err)?;
        let link = NetLink::connect(sock, &me, known, CLIENT).await.map_err(js_err)?;
        let to = NetTo { addr, me, speaker: link.speaker };
        let m = Mcu::new(link, ev.clone(), false).await.map_err(js_err)?;
        Ok(Speaker::with(Conn::Net(m), ev, events, Some(to)))
    }

    /// Over the network: the speaker's key (hex), to remember with its address; undefined over USB.
    #[wasm_bindgen(js_name = speakerKey)]
    pub fn speaker_key(&self) -> Option<String> {
        self.net.borrow().as_ref().map(|n| zd1_host::hex(&n.speaker))
    }

    /// "usb" or "net".
    pub fn link(&self) -> String {
        if self.net.borrow().is_some() { "net" } else { "usb" }.into()
    }

    /// The firmware's HELLO: { fw, built, board, proto, uptime_ms, requests } (over the network, the requests that
    /// go there).
    pub fn hello(&self) -> Result<JsValue, JsValue> {
        let h = self.m.borrow().as_ref().map(|c| on!(c, m => codec::encode_msg(&m.hello)));
        let h = h.ok_or_else(|| js_err(Error::Device("busy".into())))?.map_err(|e| js_err(e.into()))?;
        Ok(to_js(&codec::decode(&h).map_err(|e| js_err(e.into()))?))
    }

    /// One request by its name with its fields (docs/protocol/host.md); the reply as an object.
    pub async fn call(&self, name: String, fields: JsValue) -> Result<JsValue, JsValue> {
        let req = from_js(&name, &fields)?;
        let mut c = self.take().await?;
        let r = on!(&mut c, m => m.call_msg(&name, &req, true).await);
        self.put(c);
        r.map(|r| to_js(&r)).map_err(js_err)
    }

    /// Play a choice (host/core's `speaker::PLAY`: usb, aux, wifi, airplay, spotify, snapcast, standby, ...).
    pub async fn play(&self, choice: String) -> Result<(), JsValue> {
        let q = speaker::select(&choice).ok_or_else(|| js_err(Error::Failed(format!("no choice {choice}"))))?;
        let mut c = self.take().await?;
        let r = on!(&mut c, m => m.call(&q).await);
        self.put(c);
        r.map(drop).map_err(js_err)
    }

    /// One POLL: the MCU's log to events.text, the module's console (open) to events.console. The records read.
    pub async fn poll(&self) -> Result<u32, JsValue> {
        let mut c = self.take().await?;
        let (r, con) = on!(&mut c, m => (m.poll().await, m.console_take()));
        self.put(c);
        if !con.is_empty() {
            self.js.console(Uint8Array::from(con.as_slice()));
        }
        r.map(|n| n as u32).map_err(js_err)
    }

    /// Open the module's console for this page (afresh; its bytes to events.console as the page polls) or close it:
    /// ConsoleReply { open, room, lost }.
    #[wasm_bindgen(js_name = consoleOpen)]
    pub async fn console_open(&self, open: bool) -> Result<JsValue, JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => m.console_open(open).await);
        self.put(c);
        let r = r.and_then(|r| codec::encode_msg(&r).map_err(Into::into));
        Ok(to_js(&codec::decode(&r.map_err(js_err)?).map_err(|e| js_err(e.into()))?))
    }

    /// Bytes to the module's console (open), as typed.
    #[wasm_bindgen(js_name = consoleWrite)]
    pub async fn console_write(&self, data: Vec<u8>) -> Result<(), JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => m.console_write(&data).await);
        self.put(c);
        r.map_err(js_err)
    }

    /// The State record (the module OS's settings and status) as an object: its fields, `msg` "State".
    pub async fn state(&self) -> Result<JsValue, JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => settings::state_msg(m).await);
        self.put(c);
        r.map(|m| to_js(&m)).map_err(js_err)
    }

    /// A list record of the module OS by its name (`Record`): `Scan` (the networks around: { ssid, signal (-dBm),
    /// security }, the strongest first; each read starts a new scan, so a fresh radio's first answer is empty),
    /// `Networks` (the known ones, preferred first: { ssid, security }), `Servers` (snapservers over mDNS: { name,
    /// host, port }; each read looks again), `Pairings` (AirPlay's: { id, admin }), `Controllers` (the clients paired
    /// over the network: { id, name, since }). Its entries as objects; a record the module can't make fails with why
    /// (no radio, no address).
    pub async fn list(&self, record: String) -> Result<JsValue, JsValue> {
        let r =
            codec::enum_value("Record", &record).ok_or_else(|| js_err(Error::Failed(format!("no record {record}"))))?;
        let mut c = self.take().await?;
        let b = on!(&mut c, m => settings::record(m, spec::Record::from(r)).await);
        self.put(c);
        let l = codec::decode_list(&b.map_err(js_err)?).map_err(|e| js_err(Error::Codec(e)))?;
        let a = Array::new();
        for e in &l {
            a.push(&to_js(e));
        }
        Ok(a.into())
    }

    /// The settings that differ from their defaults, as KEY=VALUE lines (no secrets): a file to import later.
    #[wasm_bindgen(js_name = exportSettings)]
    pub async fn export_settings(&self) -> Result<String, JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => settings::export(m).await);
        self.put(c);
        r.map_err(js_err)
    }

    /// Settings from an export's lines, each written: how many; the ones refused in the error.
    #[wasm_bindgen(js_name = importSettings)]
    pub async fn import_settings(&self, text: String) -> Result<u32, JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => settings::import(m, &text).await);
        self.put(c);
        r.map(|n| n as u32).map_err(js_err)
    }

    /// Change a setting ("" = its default): the value the module has now. Over the network a change that can cut the
    /// page off (`spec.provisional`) is kept once the page connects again: `reachAgain` after the last of them.
    #[wasm_bindgen(js_name = setSetting)]
    pub async fn set_setting(&self, key: String, value: String) -> Result<String, JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => settings::set(m, &key, &value).await);
        self.put(c);
        r.map_err(js_err)
    }

    /// Join a Wi-Fi network: WPA2 with a passphrase (turned into the key here), or open (none).
    #[wasm_bindgen(js_name = joinWifi)]
    pub async fn join_wifi(&self, ssid: String, passphrase: Option<String>) -> Result<(), JsValue> {
        let mut c = self.take().await?;
        let how = passphrase.as_deref().map_or(settings::Wifi::Open, settings::Wifi::Wpa2);
        let r = on!(&mut c, m => settings::join(m, &ssid, how).await);
        self.put(c);
        r.map_err(js_err)
    }

    #[wasm_bindgen(js_name = wifiOff)]
    pub async fn wifi_off(&self) -> Result<(), JsValue> {
        let mut c = self.take().await?;
        let r = on!(&mut c, m => settings::wifi_off(m).await);
        self.put(c);
        r.map_err(js_err)
    }

    /// Over the network, after a change that can cut the page off (`provisional`): a new session, at this address or
    /// `also` (the address a change set), once the change has taken effect; the speaker keeps the change once it has
    /// one, else it goes back by itself (docs/protocol/net.md). The address it answered at (the connection is there
    /// now); an error when it didn't within the time.
    #[wasm_bindgen(js_name = reachAgain)]
    pub async fn reach_again(&self, also: Option<String>) -> Result<String, JsValue> {
        let Some(to) = self.net.borrow().clone() else { return Ok(String::new()) };
        let mut at = vec![to.addr.clone()];
        at.extend(also.filter(|a| *a != to.addr));
        drop(self.take().await?); // the session from before: it doesn't count
        let ev = self.ev.clone();
        let connect = async |a: &str| NetTo { addr: a.into(), ..to.clone() }.open(ev.clone()).await;
        match zd1_host::net::reach_again(&at, &*self.ev, connect).await {
            Ok((a, m)) => {
                self.put(Conn::Net(m));
                *self.net.borrow_mut() = Some(NetTo { addr: a.clone(), ..to });
                Ok(a)
            }
            Err(e) => {
                self.closed.set(true);
                Err(js_err(e))
            }
        }
    }

    /// The module's system into the slot that doesn't run, then started once (`start`): { outcome, slot, now }, the
    /// outcome "installed" (not started), "running" (the good slot now) or "fellBack" (the new slot didn't get ready:
    /// the module is on `now`); slots as "a", "b". Over the network the image must be signed (the release's
    /// `system-VERSION.signed`) and is always started; the page's connection goes with the restart and comes back by
    /// itself.
    #[wasm_bindgen(js_name = updateSystem)]
    pub async fn update_system(&self, image: Vec<u8>, start: bool) -> Result<JsValue, JsValue> {
        let c = self.take().await?;
        let to = self.net.borrow().clone();
        let r = match (c, &to) {
            (Conn::Usb(mut m), _) => {
                // over USB the image itself (the MCU's path takes any): a signed one without its header
                let image = image::signed_system(&image).map_or(&image[..], |(_, at)| &image[at..]);
                let r = update::system(&mut m, image, &update::Options { start, ..Default::default() }).await;
                self.put(Conn::Usb(m));
                r
            }
            (Conn::Net(m), Some(to)) => {
                let ev = self.ev.clone();
                match update::system_over_network(m, &image, async || to.open(ev.clone()).await).await {
                    Ok((o, m)) => {
                        self.put(Conn::Net(m));
                        Ok(o)
                    }
                    Err(e) => {
                        self.closed.set(true);
                        Err(e)
                    }
                }
            }
            (c, None) => {
                self.put(c);
                Err(Error::Failed("no network address".into()))
            }
        };
        let o = Object::new();
        let (outcome, slot, now) = match r.map_err(js_err)? {
            update::Outcome::Installed(s) => ("installed", s, None),
            update::Outcome::Running(s) => ("running", s, Some(s)),
            update::Outcome::FellBack { slot, now } => ("fellBack", slot, Some(now)),
        };
        set(&o, "outcome", outcome);
        set(&o, "slot", slot.to_string());
        set(&o, "now", now.map_or(JsValue::UNDEFINED, |n| n.to_string().into()));
        Ok(o.into())
    }

    /// The module's boot image (our .bcd: stub, kernel, rescue) through the bootloader's relay, then our firmware
    /// started again, which has the new boot image's rescue install itself into a slot (`install::boot_image`). This
    /// connection ends: connect again afterwards. { os, outcome, slot, now } ([`outcome`]). USB only.
    #[wasm_bindgen(js_name = updateBoot)]
    pub async fn update_boot(&self, bcd: Vec<u8>) -> Result<JsValue, JsValue> {
        let m = self.usb().await?; // it asks for DFU and leaves USB: this connection ends
        self.closed.set(true);
        let d = install::boot_image(&NusbBus, Some(m), &bcd, self.ev.clone()).await.map_err(js_err)?;
        let o = outcome(&d.outcome);
        set(&o, "os", d.info.mod_os);
        Ok(o.into())
    }

    /// The boot image's system into a slot, asked for (`install::system_from_rescue`, a rescue with no system in a
    /// slot): the rescue started once installs itself into the slot that isn't the good one, which then starts once.
    /// { outcome, slot, now } ([`outcome`]). USB only.
    #[wasm_bindgen(js_name = installFromRescue)]
    pub async fn install_from_rescue(&self) -> Result<JsValue, JsValue> {
        let mut m = self.usb().await?;
        let r = install::system_from_rescue(&mut m).await;
        self.put(Conn::Usb(m));
        Ok(outcome(&r.map_err(js_err)?).into())
    }

    /// The module's boot image (our .bcd) through BL2's XModem, for a module in BL2 with no image to start (a boot
    /// image update that stopped: `install::boot_image_in_bl2`); then its system from the rescue, as after the relay.
    /// The connection stays. { os, outcome, slot, now } ([`outcome`]). USB only.
    #[wasm_bindgen(js_name = updateBootInBl2)]
    pub async fn update_boot_in_bl2(&self, bcd: Vec<u8>) -> Result<JsValue, JsValue> {
        let mut m = self.usb().await?;
        let r = install::boot_image_in_bl2(&mut m, &bcd).await;
        self.put(Conn::Usb(m));
        let d = r.map_err(js_err)?;
        let o = outcome(&d.outcome);
        set(&o, "os", d.info.mod_os);
        Ok(o.into())
    }

    /// Our MCU firmware onto the speaker: into DFU (the page needs the bootloader granted: wantsDevice), a backup
    /// (events.backup: kept before anything is written), flash, verify, start, check. This connection ends: connect
    /// again afterwards. { fw, built, rcause, stages, matches }. USB only (over the network the MCU follows its OS).
    #[wasm_bindgen(js_name = flashMcu)]
    pub async fn flash_mcu(&self, name: String, image: Vec<u8>) -> Result<JsValue, JsValue> {
        let img = McuImage::parse(&name, &image).map_err(js_err)?;
        let m = self.usb().await?; // it asks for DFU and leaves USB: this connection ends
        self.closed.set(true);
        flash_js(Some(m), &img, &self.ev, &self.js).await
    }
}

#[wasm_bindgen]
impl Speaker {
    /// The whole install from here (`Bootloader.install`, which gets into the bootloader by itself): for a speaker
    /// whose install stopped with the module on the stock software. This connection ends: connect again afterwards.
    /// USB only.
    pub async fn install(
        &self,
        bcd: Option<Vec<u8>>,
        mcu_name: String,
        mcu: Vec<u8>,
        system: Option<Vec<u8>>,
    ) -> Result<JsValue, JsValue> {
        let m = self.usb().await?;
        self.closed.set(true);
        drop(m); // the install opens it again, to ask for the bootloader
        install_js(bcd, &mcu_name, &mcu, system.as_deref(), &self.ev, &self.js).await
    }
}

/// `install::install` (the backup to events.backup): { fw, slot, os }.
async fn install_js(
    bcd: Option<Vec<u8>>,
    mcu_name: &str,
    mcu: &[u8],
    system: Option<&[u8]>,
    ev: &Rc<dyn Events>,
    js: &JsEvents,
) -> Result<JsValue, JsValue> {
    let img = McuImage::parse(mcu_name, mcu).map_err(js_err)?;
    let images = install::Images { bcd: bcd.as_deref(), mcu: &img, system };
    let r = install::install(&NusbBus, &images, ev.clone(), async |b| keep(js, b).await).await.map_err(js_err)?;
    let o = Object::new();
    set(&o, "fw", r.firmware.fw);
    set(&o, "slot", r.slot.to_string());
    set(&o, "os", r.os);
    Ok(o.into())
}

/// `flash::firmware`, the backup to events.backup; the report as { fw, built, rcause, stages, matches }.
async fn flash_js(
    m: Option<Mcu<NusbDev>>,
    img: &McuImage,
    ev: &Rc<dyn Events>,
    js: &JsEvents,
) -> Result<JsValue, JsValue> {
    let r = flash::firmware(&NusbBus, m, img, ev.clone(), async |b| keep(js, b).await).await.map_err(js_err)?;
    Ok(report_js(r))
}

fn report_js(r: flash::Report) -> JsValue {
    let o = Object::new();
    set(&o, "fw", r.fw);
    set(&o, "built", r.built);
    set(&o, "rcause", r.rcause);
    set(&o, "stages", r.stages.into_iter().map(JsValue::from).collect::<Array>());
    set(&o, "matches", r.matches);
    o.into()
}

impl Speaker {
    fn with(c: Conn, ev: Rc<dyn Events>, js: JsEvents, net: Option<NetTo>) -> Speaker {
        Speaker { m: Rc::new(RefCell::new(Some(c))), closed: Cell::new(false), ev, js, net: RefCell::new(net) }
    }

    /// The connection for one operation (another one running: wait for it; the page polls the log meanwhile).
    async fn take(&self) -> Result<Conn, JsValue> {
        loop {
            if self.closed.get() {
                return Err(js_err(Error::Usb("this connection has ended (connect again)", UsbError::Gone)));
            }
            if let Some(c) = self.m.borrow_mut().take() {
                return Ok(c);
            }
            zd1_host::sleep(10).await;
        }
    }

    /// The connection for an operation only USB has (DFU, the relay).
    async fn usb(&self) -> Result<Mcu<NusbDev>, JsValue> {
        match self.take().await? {
            Conn::Usb(m) => Ok(m),
            c => {
                self.put(c);
                Err(js_err(Error::Failed("over USB only: connect the speaker with a cable for this".into())))
            }
        }
    }

    fn put(&self, c: Conn) {
        *self.m.borrow_mut() = Some(c);
    }
}

/// The speaker in its bootloader (DFU: our firmware isn't running, or doesn't start). Each operation opens it.
#[wasm_bindgen]
pub struct Bootloader {
    ev: Rc<dyn Events>,
    js: JsEvents,
}

#[wasm_bindgen]
impl Bootloader {
    /// The bootloader among the devices the page may use (undefined: not there, or not granted).
    pub async fn connect(events: JsEvents) -> Result<Option<Bootloader>, JsValue> {
        let ev: Rc<dyn Events> = Rc::new(Ev(events.clone().unchecked_into()));
        let present = NusbBus.present(dfu::VID, dfu::PID).await.map_err(js_err)?;
        Ok(present.then(|| Bootloader { ev, js: events }))
    }

    /// What it says: { module } (the network module's state as the bootloader sees it), and `boot`: the stages our
    /// firmware's last boots reached ([{ boot, stage, rcause }], the stage by its name; empty: it never started).
    pub async fn state(&self) -> Result<JsValue, JsValue> {
        let mut d = self.open().await?;
        d.make_idle().await.map_err(js_err)?;
        let s = d.status().await.map_err(js_err)?;
        let log = flash::read_boot_log(&mut d, &*self.ev).await.map_err(js_err)?;
        let o = Object::new();
        set(&o, "module", dfu::coproc_status_name(s.coproc));
        let boot = Array::new();
        for st in log {
            let e = Object::new();
            set(&e, "boot", st.boot);
            set(&e, "stage", flash::stage_name(st.stage));
            set(&e, "rcause", st.rcause);
            boot.push(&e);
        }
        set(&o, "boot", boot);
        Ok(o.into())
    }

    /// Start the firmware on it, and what that says ({ fw, built, rcause, stages, matches }). Connect again afterwards.
    pub async fn start(&self) -> Result<JsValue, JsValue> {
        Ok(report_js(flash::start(&NusbBus, self.ev.clone()).await.map_err(js_err)?))
    }

    /// Our MCU firmware onto it: a backup (events.backup), flash, verify, start, check (`Speaker.flashMcu` from here).
    #[wasm_bindgen(js_name = flashMcu)]
    pub async fn flash_mcu(&self, name: String, image: Vec<u8>) -> Result<JsValue, JsValue> {
        let img = McuImage::parse(&name, &image).map_err(js_err)?;
        flash_js(None, &img, &self.ev, &self.js).await
    }

    /// Our firmware on both boards (host/core's `install`, docs/web.md): the module's boot image through the relay
    /// (`bcd`; undefined: it is there already), the MCU firmware (a backup first: events.backup), the module's first
    /// boot, then the system: the boot image's own (its rescue installs itself), else `system` into a slot. Phases on
    /// events.phase. { fw, slot, os }; connect again afterwards.
    pub async fn install(
        &self,
        bcd: Option<Vec<u8>>,
        mcu_name: String,
        mcu: Vec<u8>,
        system: Option<Vec<u8>>,
    ) -> Result<JsValue, JsValue> {
        install_js(bcd, &mcu_name, &mcu, system.as_deref(), &self.ev, &self.js).await
    }

    /// The stock firmware back on both boards (host/core's `rollback`): the stock `.bcd` through the relay, then the
    /// stock MCU firmware (a .hex, or a backup of it), a backup of ours first (events.backup). The stock
    /// firmware has no host link: the page sees nothing of the speaker afterwards.
    pub async fn rollback(&self, bcd: Vec<u8>, mcu_name: String, mcu: Vec<u8>) -> Result<(), JsValue> {
        let img = McuImage::parse(&mcu_name, &mcu).map_err(js_err)?;
        install::rollback(&NusbBus, &bcd, &img, self.ev.clone(), async |b| keep(&self.js, b).await)
            .await
            .map_err(js_err)
    }
}

impl Bootloader {
    async fn open(&self) -> Result<dfu::Dfu<NusbDev>, JsValue> {
        NusbBus
            .open(dfu::VID, dfu::PID, Iface::Number(dfu::IFACE))
            .await
            .map_err(js_err)?
            .map(dfu::Dfu::new)
            .ok_or_else(|| js_err(Error::Device("the speaker's bootloader has left USB".into())))
    }
}
