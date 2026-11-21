//! `navigator.usb` for the polyfill: the speaker's devices through nusb, one request here for each WebUSB call the
//! page's nusb makes (host/core under WebAssembly). The devices are granted from the start, and they are the only ones:
//! our firmware and the MCU's DFU bootloader. `ZD1DFU_EMU=host:port`: the emulator's bridge instead (EP0 only, as
//! host/core's `EmuBus`).
//!
//! Each device's requests run in order on lanes: one for its control endpoint and its state (open, configuration,
//! interfaces), one for each endpoint it transfers on, so a transfer that waits doesn't hold up the others, as in a
//! browser.

use crate::{Answer, Page, Reply};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use nusb::descriptors::{ConfigurationDescriptor, DeviceDescriptor, TransferType};
use nusb::transfer::{
    Buffer, Bulk, Completion, ControlIn, ControlOut, ControlType, Direction, In, Interrupt, Out, Recipient,
    TransferError,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use zd1_host::{dfu, spec};

/// The devices the page may use: our firmware, the MCU's bootloader.
const GRANTED: [(u16, u16); 2] = [(spec::usb::VID, spec::usb::PID), (dfu::VID, dfu::PID)];

/// A control transfer's time here (WebUSB has none; a DFU erase is the longest wait, seconds).
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

type Res<T> = Result<T, (&'static str, String)>;

fn err<T>(name: &'static str, msg: impl Into<String>) -> Res<T> {
    Err((name, msg.into()))
}

pub struct Usb {
    page: Page,
    rt: Handle,
    devs: Mutex<HashMap<u32, Arc<Dev>>>,
    next: AtomicU32,
    emu: Option<String>,
}

/// Where a device is: the computer's USB, or the emulator's bridge (its connection, the descriptors it gave).
enum Src {
    Usb(nusb::DeviceInfo),
    Emu(Arc<Mutex<TcpStream>>, Vec<u8>, Vec<u8>),
}

struct Dev {
    key: u32,
    src: Src,
    open: Mutex<Option<Open>>,
    lanes: Mutex<HashMap<u8, mpsc::UnboundedSender<Job>>>,
}

/// A request waiting on a lane: its op, its arguments, where its answer goes.
type Job = (String, Value, Reply);

/// The bridge's device as probed: its connection, the device descriptor, the configuration descriptor.
type Probed = (TcpStream, Vec<u8>, Vec<u8>);

/// An opened device: its interfaces claimed, the endpoints in use, and a signal that ends what waits on them.
struct Open {
    dev: Option<nusb::Device>,
    intfs: HashMap<u8, nusb::Interface>,
    eps: HashMap<u8, Arc<tokio::sync::Mutex<Ep>>>,
    closed: watch::Sender<bool>,
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = self.closed.send(true);
    }
}

enum Ep {
    BulkIn(nusb::Endpoint<Bulk, In>),
    BulkOut(nusb::Endpoint<Bulk, Out>),
    IntIn(nusb::Endpoint<Interrupt, In>),
    IntOut(nusb::Endpoint<Interrupt, Out>),
}

macro_rules! each_ep {
    ($ep:expr, $e:ident => $body:expr) => {
        match $ep {
            Ep::BulkIn($e) => $body,
            Ep::BulkOut($e) => $body,
            Ep::IntIn($e) => $body,
            Ep::IntOut($e) => $body,
        }
    };
}

impl Usb {
    pub fn new(page: Page, rt: &Handle) -> Arc<Usb> {
        let emu = std::env::var("ZD1DFU_EMU").ok().filter(|s| !s.is_empty());
        if let Some(e) = &emu {
            eprintln!("zd1-app: USB is the emulator's bridge at {e}");
        }
        Arc::new(Usb { page, rt: rt.clone(), devs: Mutex::default(), next: AtomicU32::new(1), emu })
    }

    /// The speaker coming and going (`connect`, `disconnect` on `navigator.usb`).
    pub fn watch(self: &Arc<Self>) {
        if self.emu.is_some() {
            return; // the bridge has no hotplug: the page's waits look again
        }
        let this = self.clone();
        self.rt.spawn(async move {
            use futures_util::StreamExt;
            let mut w = match nusb::watch_devices() {
                Ok(w) => w,
                Err(e) => return eprintln!("zd1-app: no USB hotplug: {e}"),
            };
            while let Some(ev) = w.next().await {
                match ev {
                    nusb::hotplug::HotplugEvent::Connected(info) if granted(info.vendor_id(), info.product_id()) => {
                        let d = this.known(info);
                        this.page.event("usb.connect", d.info());
                    }
                    nusb::hotplug::HotplugEvent::Disconnected(id) => {
                        let gone = {
                            let mut devs = this.devs.lock().unwrap();
                            let k = devs
                                .iter()
                                .find(|(_, d)| matches!(&d.src, Src::Usb(i) if i.id() == id))
                                .map(|(k, _)| *k);
                            k.and_then(|k| devs.remove(&k))
                        };
                        if let Some(d) = gone {
                            d.open.lock().unwrap().take();
                            this.page.event("usb.disconnect", json!({ "key": d.key }));
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    /// A page (re)loading: what the last one had open goes, as a browser closes a tab's devices.
    pub fn close_all(&self) {
        for d in self.devs.lock().unwrap().values() {
            d.open.lock().unwrap().take();
        }
    }

    /// The device for this nusb device: the one known, or a new key.
    fn known(&self, info: nusb::DeviceInfo) -> Arc<Dev> {
        let mut devs = self.devs.lock().unwrap();
        if let Some(d) = devs.values().find(|d| matches!(&d.src, Src::Usb(i) if i.id() == info.id())) {
            return d.clone();
        }
        let d = Dev::new(self.next.fetch_add(1, Ordering::Relaxed), Src::Usb(info));
        devs.insert(d.key, d.clone());
        d
    }

    /// A request from the polyfill: `list` here, the rest on the device's lanes.
    pub fn request(self: &Arc<Self>, op: &str, m: Value, reply: Reply, rt: &Handle) {
        if op == "list" {
            let this = self.clone();
            rt.spawn(async move { reply.send(this.list().await.map(Value::Array)) });
            return;
        }
        let key = m["key"].as_u64().unwrap_or(0) as u32;
        let Some(d) = self.devs.lock().unwrap().get(&key).cloned() else {
            return reply.send(err("NotFoundError", "the device is gone"));
        };
        let lane = match op {
            "transferIn" | "transferOut" | "clearHalt" => {
                let n = m["endpoint"].as_u64().unwrap_or(0) as u8 & 0x7F;
                let dir_in = op == "transferIn" || m["direction"] == "in";
                n | if dir_in { 0x80 } else { 0 }
            }
            _ => 0, // the control endpoint, and the device's state
        };
        let tx = d
            .lanes
            .lock()
            .unwrap()
            .entry(lane)
            .or_insert_with(|| {
                let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
                let d = Arc::downgrade(&d); // the lane ends with its device
                rt.spawn(async move {
                    while let Some((op, m, reply)) = rx.recv().await {
                        match d.upgrade() {
                            Some(d) => reply.send(d.run(&op, &m).await),
                            None => reply.send(err("NotFoundError", "the device is gone")),
                        }
                    }
                });
                tx
            })
            .clone();
        let _ = tx.send((op.to_string(), m, reply));
    }

    /// The granted devices that are here (`getDevices`).
    async fn list(&self) -> Res<Vec<Value>> {
        if let Some(addr) = &self.emu {
            return self.list_emu(addr).await;
        }
        let found = nusb::list_devices().await.map_err(|e| ("NotFoundError", e.to_string()))?;
        let here: Vec<_> = found.filter(|i| granted(i.vendor_id(), i.product_id())).collect();
        let ids: Vec<_> = here.iter().map(|i| i.id()).collect();
        self.devs.lock().unwrap().retain(|_, d| matches!(&d.src, Src::Usb(i) if ids.contains(&i.id())));
        Ok(here.into_iter().map(|i| self.known(i).info()).collect())
    }

    /// The bridge's device, probed as host/core's `EmuBus` does unless one is open: a bus reset, its descriptors, and
    /// for our firmware SET_ADDRESS and SET_CONFIGURATION 1, as a host OS gives it.
    async fn list_emu(&self, addr: &str) -> Res<Vec<Value>> {
        let open = self.devs.lock().unwrap().values().find(|d| d.open.lock().unwrap().is_some()).cloned();
        if let Some(d) = open {
            return Ok(vec![d.info()]);
        }
        let addr = addr.to_string();
        let probed = tokio::task::spawn_blocking(move || -> std::io::Result<Option<Probed>> {
            let mut s = TcpStream::connect(&addr)?;
            s.set_read_timeout(Some(Duration::from_secs(300)))?;
            emu_x(&mut s, b"R")?;
            let get = |s: &mut TcpStream, ty: u16, len: u16| emu_setup(s, 0x80, 6, ty << 8, 0, len, &[]);
            let Some(dd) = get(&mut s, 1, 18)?.filter(|d| d.len() >= 18) else { return Ok(None) };
            let total = get(&mut s, 2, 9)?.and_then(|h| h.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]])));
            let cfg = match total {
                Some(t) => get(&mut s, 2, t)?.unwrap_or_default(),
                None => Vec::new(),
            };
            if u16::from_le_bytes([dd[10], dd[11]]) == spec::usb::PID {
                for (req, val) in [(5u8, 1u16), (9, 1)] {
                    emu_setup(&mut s, 0x00, req, val, 0, 0, &[])?;
                }
            }
            Ok(Some((s, dd, cfg)))
        })
        .await
        .map_err(|e| ("NotFoundError", e.to_string()))?
        .unwrap_or_else(|e| {
            crate::log(&format!("the emulator's USB: {e}")); // no emulator: no device
            None
        });
        let mut devs = self.devs.lock().unwrap();
        devs.clear();
        let Some((s, dd, cfg)) = probed else { return Ok(vec![]) };
        if !granted(u16::from_le_bytes([dd[8], dd[9]]), u16::from_le_bytes([dd[10], dd[11]])) {
            return Ok(vec![]);
        }
        let d = Dev::new(self.next.fetch_add(1, Ordering::Relaxed), Src::Emu(Arc::new(Mutex::new(s)), dd, cfg));
        devs.insert(d.key, d.clone());
        Ok(vec![d.info()])
    }
}

fn granted(vid: u16, pid: u16) -> bool {
    GRANTED.contains(&(vid, pid))
}

impl Dev {
    fn new(key: u32, src: Src) -> Arc<Dev> {
        Arc::new(Dev { key, src, open: Mutex::default(), lanes: Mutex::default() })
    }

    /// The device as WebUSB describes it (`USBDevice`'s fields, its key): its configurations from the descriptors once
    /// it is open, before that from what the system's enumeration says.
    fn info(&self) -> Value {
        let mut v = match &self.src {
            Src::Usb(i) => {
                let opened = self.open.lock().unwrap().as_ref().and_then(|o| o.dev.clone());
                let (configuration, configurations) = match &opened {
                    Some(d) => (
                        d.active_configuration().ok().map(|c| config(&c)),
                        d.configurations().map(|c| config(&c)).collect::<Vec<_>>(),
                    ),
                    None => {
                        // the system's view before open: the active configuration's first alternates
                        let intfs: Vec<_> = i
                            .interfaces()
                            .map(|f| {
                                json!({ "interfaceNumber": f.interface_number(), "alternates": [{
                                    "alternateSetting": 0, "interfaceClass": f.class(),
                                    "interfaceSubclass": f.subclass(), "interfaceProtocol": f.protocol(),
                                    "interfaceName": f.interface_string(), "endpoints": [] }] })
                            })
                            .collect();
                        let c = (!intfs.is_empty()).then(
                            || json!({ "configurationValue": 1, "configurationName": null, "interfaces": intfs }),
                        );
                        (c.clone(), c.into_iter().collect())
                    }
                };
                json!({
                    "vendorId": i.vendor_id(), "productId": i.product_id(),
                    "deviceClass": i.class(), "deviceSubclass": i.subclass(), "deviceProtocol": i.protocol(),
                    "usb": i.usb_version(), "device": i.device_version(),
                    "manufacturerName": i.manufacturer_string(), "productName": i.product_string(),
                    "serialNumber": i.serial_number(),
                    "configuration": configuration, "configurations": configurations,
                })
            }
            Src::Emu(_, dd, cfg) => {
                let d = DeviceDescriptor::new(dd);
                let c = ConfigurationDescriptor::new(cfg).map(|c| config(&c));
                json!({
                    "vendorId": d.as_ref().map(|d| d.vendor_id()), "productId": d.as_ref().map(|d| d.product_id()),
                    "deviceClass": d.as_ref().map(|d| d.class()), "deviceSubclass": d.as_ref().map(|d| d.subclass()),
                    "deviceProtocol": d.as_ref().map(|d| d.protocol()),
                    "usb": d.as_ref().map(|d| d.usb_version()), "device": d.as_ref().map(|d| d.device_version()),
                    "manufacturerName": null, "productName": null, "serialNumber": null,
                    "configuration": c, "configurations": c.iter().collect::<Vec<_>>(),
                })
            }
        };
        v["key"] = json!(self.key);
        v
    }

    /// One request on this device, in its lane's order.
    async fn run(self: &Arc<Self>, op: &str, m: &Value) -> Answer {
        match op {
            "open" => self.op_open().await,
            "close" => {
                self.open.lock().unwrap().take();
                Ok(Value::Null)
            }
            "forget" => Ok(Value::Null),
            "selectConfiguration" => {
                let v = m["value"].as_u64().unwrap_or(1) as u8;
                if let Some(d) = self.device()? {
                    d.set_configuration(v).await.map_err(|e| ("InvalidStateError", e.to_string()))?;
                }
                Ok(self.info())
            }
            "claimInterface" => {
                let n = m["interface"].as_u64().unwrap_or(0) as u8;
                if let Some(d) = self.device()? {
                    let i = d.claim_interface(n).await.map_err(|e| ("NetworkError", e.to_string()))?;
                    self.with_open(|o| o.intfs.insert(n, i))?;
                }
                Ok(Value::Null)
            }
            "releaseInterface" => {
                let n = m["interface"].as_u64().unwrap_or(0) as u8;
                let i = self.with_open(|o| {
                    let i = o.intfs.remove(&n)?;
                    let addrs: Vec<u8> = i.descriptor().map(|a| a.endpoints().map(|e| e.address()).collect())?;
                    o.eps.retain(|a, _| !addrs.contains(a));
                    Some(i)
                })?;
                if let Some(i) = i {
                    let _ = i.release().await; // Busy while a transfer still holds it: it goes when that ends
                }
                Ok(Value::Null)
            }
            "selectAlternateInterface" => {
                let n = m["interface"].as_u64().unwrap_or(0) as u8;
                let alt = m["alternate"].as_u64().unwrap_or(0) as u8;
                if self.device()?.is_some() {
                    let i = self.intf(n)?;
                    i.set_alt_setting(alt).await.map_err(|e| ("NetworkError", e.to_string()))?;
                }
                Ok(Value::Null)
            }
            "reset" => {
                if let Some(d) = self.device()? {
                    d.reset().await.map_err(|e| ("NetworkError", e.to_string()))?;
                }
                Ok(Value::Null)
            }
            "controlTransferIn" => self.control(m, true).await,
            "controlTransferOut" => self.control(m, false).await,
            "transferIn" | "transferOut" => self.transfer(op == "transferIn", m).await,
            "clearHalt" => {
                let addr =
                    (m["endpoint"].as_u64().unwrap_or(0) as u8 & 0x7F) | if m["direction"] == "in" { 0x80 } else { 0 };
                let ep = self.ep(addr)?;
                let mut ep = ep.lock().await;
                each_ep!(&mut *ep, e => e.clear_halt().await).map_err(|e| ("NetworkError", e.to_string()))?;
                Ok(Value::Null)
            }
            _ => err("NotSupportedError", format!("USBDevice has no {op} here")),
        }
    }

    async fn op_open(self: &Arc<Self>) -> Answer {
        if self.open.lock().unwrap().is_some() {
            return Ok(self.info());
        }
        let dev = match &self.src {
            Src::Usb(i) => Some(i.open().await.map_err(|e| ("NetworkError", format!("can't open the device: {e}")))?),
            Src::Emu(..) => None,
        };
        let (closed, _) = watch::channel(false);
        *self.open.lock().unwrap() = Some(Open { dev, intfs: HashMap::new(), eps: HashMap::new(), closed });
        Ok(self.info())
    }

    fn with_open<T>(&self, f: impl FnOnce(&mut Open) -> T) -> Res<T> {
        match self.open.lock().unwrap().as_mut() {
            Some(o) => Ok(f(o)),
            None => err("InvalidStateError", "the device is not open"),
        }
    }

    /// The nusb device, None on the emulator's bridge; an error if not open.
    fn device(&self) -> Res<Option<nusb::Device>> {
        self.with_open(|o| o.dev.clone())
    }

    fn intf(&self, n: u8) -> Res<nusb::Interface> {
        self.with_open(|o| o.intfs.get(&n).cloned())?
            .map_or_else(|| err("InvalidStateError", format!("interface {n} is not claimed")), Ok)
    }

    async fn control(&self, m: &Value, dir_in: bool) -> Answer {
        let s = &m["setup"];
        let control_type = match s["requestType"].as_str() {
            Some("standard") => ControlType::Standard,
            Some("class") => ControlType::Class,
            Some("vendor") => ControlType::Vendor,
            t => return err("TypeError", format!("requestType {t:?}")),
        };
        let recipient = match s["recipient"].as_str() {
            Some("device") => Recipient::Device,
            Some("interface") => Recipient::Interface,
            Some("endpoint") => Recipient::Endpoint,
            Some("other") => Recipient::Other,
            r => return err("TypeError", format!("recipient {r:?}")),
        };
        let (request, value, index) = (
            s["request"].as_u64().unwrap_or(0) as u8,
            s["value"].as_u64().unwrap_or(0) as u16,
            s["index"].as_u64().unwrap_or(0) as u16,
        );
        let data = bytes(&m["data"])?;
        let length = m["length"].as_u64().unwrap_or(0) as u16;

        if let Src::Emu(s, ..) = &self.src {
            self.with_open(|_| ())?;
            let t = match control_type {
                ControlType::Standard => 0,
                ControlType::Class => 1,
                ControlType::Vendor => 2,
            };
            let r = match recipient {
                Recipient::Device => 0,
                Recipient::Interface => 1,
                Recipient::Endpoint => 2,
                Recipient::Other => 3,
            };
            let bm = if dir_in { 0x80 } else { 0 } | t << 5 | r;
            let (s, len) = (s.clone(), if dir_in { length } else { data.len() as u16 });
            let r = tokio::task::spawn_blocking(move || {
                emu_setup(&mut s.lock().unwrap(), bm, request, value, index, len, &data).map(|r| (r, data.len()))
            })
            .await
            .map_err(|e| ("NetworkError", e.to_string()))?
            .map_err(|e| ("NetworkError", format!("the emulator's USB: {e}")))?;
            return Ok(match (r, dir_in) {
                ((None, _), _) => json!({ "status": "stall", "bytesWritten": 0 }),
                ((Some(d), _), true) => json!({ "status": "ok", "data": B64.encode(d) }),
                ((Some(_), n), false) => json!({ "status": "ok", "bytesWritten": n }),
            });
        }

        let dev = self.device()?.expect("a device on USB");
        // the device's and its configurations' descriptors: what the system read already (Chrome's way too)
        if dir_in && control_type == ControlType::Standard && recipient == Recipient::Device && request == 6 {
            let d = match value >> 8 {
                1 => Some(dev.device_descriptor().as_bytes().to_vec()),
                2 => dev.configurations().nth((value & 0xFF) as usize).map(|c| c.as_bytes().to_vec()),
                _ => None,
            };
            if let Some(mut d) = d {
                d.truncate(length as usize);
                return Ok(json!({ "status": "ok", "data": B64.encode(d) }));
            }
        }
        // through a claimed interface (the addressed one first): Windows has no other way, macOS and Linux either way
        let intf = self.with_open(|o| {
            let wanted = (recipient == Recipient::Interface).then_some(index as u8);
            wanted.and_then(|n| o.intfs.get(&n)).or_else(|| o.intfs.values().next()).cloned()
        })?;
        let r = if dir_in {
            let c = ControlIn { control_type, recipient, request, value, index, length };
            match intf {
                Some(i) => i.control_in(c, CONTROL_TIMEOUT).await,
                None => device_control_in(&dev, c).await,
            }
            .map(|d| json!({ "status": "ok", "data": B64.encode(d) }))
        } else {
            let c = ControlOut { control_type, recipient, request, value, index, data: &data };
            match intf {
                Some(i) => i.control_out(c, CONTROL_TIMEOUT).await,
                None => device_control_out(&dev, c).await,
            }
            .map(|()| json!({ "status": "ok", "bytesWritten": data.len() }))
        };
        status(r, dir_in)
    }

    /// The endpoint at `addr` on a claimed interface's current alternate, opened once.
    fn ep(&self, addr: u8) -> Res<Arc<tokio::sync::Mutex<Ep>>> {
        self.with_open(|o| {
            if let Some(e) = o.eps.get(&addr) {
                return Ok(e.clone());
            }
            for i in o.intfs.values() {
                let Some(t) =
                    i.descriptor().and_then(|a| a.endpoints().find(|e| e.address() == addr)).map(|e| e.transfer_type())
                else {
                    continue;
                };
                let e = |r: Result<Ep, nusb::Error>| r.map_err(|e| ("InvalidAccessError", e.to_string()));
                let ep = match (t, addr & 0x80 != 0) {
                    (TransferType::Bulk, true) => e(i.endpoint::<Bulk, In>(addr).map(Ep::BulkIn))?,
                    (TransferType::Bulk, false) => e(i.endpoint::<Bulk, Out>(addr).map(Ep::BulkOut))?,
                    (TransferType::Interrupt, true) => e(i.endpoint::<Interrupt, In>(addr).map(Ep::IntIn))?,
                    (TransferType::Interrupt, false) => e(i.endpoint::<Interrupt, Out>(addr).map(Ep::IntOut))?,
                    (t, _) => return err("NotSupportedError", format!("{t:?} endpoints aren't bridged")),
                };
                let ep = Arc::new(tokio::sync::Mutex::new(ep));
                o.eps.insert(addr, ep.clone());
                return Ok(ep);
            }
            err("NotFoundError", format!("no claimed interface has endpoint {addr:02x}"))
        })?
    }

    async fn transfer(&self, dir_in: bool, m: &Value) -> Answer {
        if matches!(self.src, Src::Emu(..)) {
            return err("NotSupportedError", "the emulator's bridge carries EP0 only");
        }
        let addr = (m["endpoint"].as_u64().unwrap_or(0) as u8 & 0x7F) | if dir_in { 0x80 } else { 0 };
        let ep = self.ep(addr)?;
        let mut closed = self.with_open(|o| o.closed.subscribe())?;
        let mut ep = ep.lock().await;
        let length = m["length"].as_u64().unwrap_or(0) as usize;
        let buf = if dir_in {
            // nusb wants whole packets; more than asked for is babble, as WebUSB says
            let mps = each_ep!(&*ep, e => e.max_packet_size()).max(1);
            let mut b = Buffer::new(length.max(1).div_ceil(mps) * mps);
            b.set_requested_len(b.capacity());
            b
        } else {
            Buffer::from(bytes(&m["data"])?)
        };
        each_ep!(&mut *ep, e => e.submit(buf));
        let c: Completion = tokio::select! {
            c = async { each_ep!(&mut *ep, e => e.next_complete().await) } => c,
            () = async { let _ = closed.wait_for(|c| *c).await; } => {
                each_ep!(&mut *ep, e => e.cancel_all());
                let _ = each_ep!(&mut *ep, e => e.next_complete().await);
                return err("AbortError", "the device was closed");
            }
        };
        if dir_in {
            let (n, data) = (c.actual_len, c.buffer.into_vec());
            if c.status.is_ok() && n > length {
                return Ok(json!({ "status": "babble", "data": B64.encode(&data[..length]) }));
            }
            status(c.status.map(|()| json!({ "status": "ok", "data": B64.encode(&data[..n]) })), true)
        } else {
            status(c.status.map(|()| json!({ "status": "ok", "bytesWritten": c.actual_len })), false)
        }
    }
}

#[cfg(not(target_os = "windows"))]
async fn device_control_in(d: &nusb::Device, c: ControlIn) -> Result<Vec<u8>, TransferError> {
    d.control_in(c, CONTROL_TIMEOUT).await
}
#[cfg(not(target_os = "windows"))]
async fn device_control_out(d: &nusb::Device, c: ControlOut<'_>) -> Result<(), TransferError> {
    d.control_out(c, CONTROL_TIMEOUT).await
}
#[cfg(target_os = "windows")]
async fn device_control_in(_: &nusb::Device, _: ControlIn) -> Result<Vec<u8>, TransferError> {
    Err(TransferError::InvalidArgument) // Windows: through a claimed interface only
}
#[cfg(target_os = "windows")]
async fn device_control_out(_: &nusb::Device, _: ControlOut<'_>) -> Result<(), TransferError> {
    Err(TransferError::InvalidArgument)
}

/// A transfer's end as WebUSB gives it: a stall is a status, the device gone a NetworkError (nusb in the page: its
/// `Disconnected`), anything else an error of its own.
fn status(r: Result<Value, TransferError>, dir_in: bool) -> Answer {
    match r {
        Ok(v) => Ok(v),
        Err(TransferError::Stall) => Ok(if dir_in {
            json!({ "status": "stall", "data": "" })
        } else {
            json!({ "status": "stall", "bytesWritten": 0 })
        }),
        Err(TransferError::Disconnected) => err("NetworkError", "the device is gone"),
        Err(TransferError::Cancelled) => err("AbortError", "the transfer was cancelled"),
        Err(e) => err("OperationError", format!("transfer: {e}")),
    }
}

/// The polyfill's bytes (base64; none: empty).
fn bytes(v: &Value) -> Res<Vec<u8>> {
    match v.as_str() {
        Some(s) => B64.decode(s).map_err(|e| ("TypeError", e.to_string())),
        None => Ok(Vec::new()),
    }
}

/// A configuration as WebUSB's `USBConfiguration`.
fn config(c: &ConfigurationDescriptor) -> Value {
    let intfs: Vec<Value> = c
        .interfaces()
        .map(|i| {
            let alts: Vec<Value> = i
                .alt_settings()
                .map(|a| {
                    let eps: Vec<Value> = a
                        .endpoints()
                        .map(|e| {
                            json!({
                                "endpointNumber": e.address() & 0x7F,
                                "direction": if e.direction() == Direction::In { "in" } else { "out" },
                                "type": match e.transfer_type() {
                                    TransferType::Bulk => "bulk",
                                    TransferType::Interrupt => "interrupt",
                                    TransferType::Isochronous => "isochronous",
                                    TransferType::Control => "control",
                                },
                                "packetSize": e.max_packet_size(),
                            })
                        })
                        .collect();
                    json!({
                        "alternateSetting": a.alternate_setting(), "interfaceClass": a.class(),
                        "interfaceSubclass": a.subclass(), "interfaceProtocol": a.protocol(),
                        "interfaceName": null, "endpoints": eps,
                    })
                })
                .collect();
            json!({ "interfaceNumber": i.interface_number(), "alternates": alts })
        })
        .collect();
    json!({ "configurationValue": c.configuration_value(), "configurationName": null, "interfaces": intfs })
}

// ---- the emulator's bridge: 'R' bus reset | 'S' setup[8] + OUT data; answers 'A', 'D' len16 data, else a stall ----

fn emu_x(s: &mut TcpStream, msg: &[u8]) -> std::io::Result<Option<Vec<u8>>> {
    s.write_all(msg)?;
    let mut k = [0u8; 1];
    s.read_exact(&mut k)?;
    match k[0] {
        b'A' => Ok(Some(Vec::new())),
        b'D' => {
            let mut n = [0u8; 2];
            s.read_exact(&mut n)?;
            let mut d = vec![0u8; u16::from_le_bytes(n) as usize];
            s.read_exact(&mut d)?;
            Ok(Some(d))
        }
        _ => Ok(None),
    }
}

fn emu_setup(
    s: &mut TcpStream,
    bm: u8,
    req: u8,
    value: u16,
    index: u16,
    len: u16,
    data: &[u8],
) -> std::io::Result<Option<Vec<u8>>> {
    let mut m = vec![b'S', bm, req];
    for v in [value, index, len] {
        m.extend(v.to_le_bytes());
    }
    m.extend(data);
    emu_x(s, &m)
}
