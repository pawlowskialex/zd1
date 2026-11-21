//! A link to the speaker's host link: how a request gets there and its records come back. Over USB, every [`Usb`]
//! device is one (CALL, POLL and HELLO control transfers to the host link's interface: docs/protocol/host.md); over
//! the network, [`crate::net::NetLink`] (our module OS, docs/protocol/net.md). [`crate::Mcu`] runs the host link over
//! either; a flow that needs USB itself (the bulk pipe, DFU) asks for a [`Usb`] device instead.

use crate::spec::{msg, usb};
use crate::usb::{Setup, Usb, UsbError};
use crate::{Error, Result, codec};

/// bmRequestType: vendor, to the interface.
const VENDOR_IN: u8 = 0xC1;
const VENDOR_OUT: u8 = 0x41;

#[allow(async_fn_in_trait)]
pub trait Link {
    /// The firmware's `HelloReply`, encoded.
    async fn hello(&self) -> Result<Vec<u8>>;
    /// Send a request, `seq` its sequence number; false: the other end runs another one still (send it again after a
    /// poll).
    async fn call(&self, seq: u16, request: &[u8]) -> Result<bool>;
    /// The records that came since the last poll (kind, u16 length, payload: `spec::record`).
    async fn poll(&self) -> Result<Vec<u8>>;
    /// Wait, in the other end's time.
    async fn sleep(&self, ms: u32);
    /// How many times slower the other end's time runs than ours (the emulator's).
    fn slow(&self) -> u32 {
        1
    }
    /// What gets a refused gated request through, after a press of ON/OFF on the speaker: `Unlock` over USB (the
    /// gate, for 10 minutes), `Pair` over the network (this client, for good). Its name and the request.
    fn unlock(&self) -> (&'static str, Vec<u8>) {
        ("Unlock", codec::encode_msg(&msg::Unlock).expect("Unlock encodes"))
    }
    /// The speaker over the network (some requests stay USB's: the spec's `net` marker).
    fn network(&self) -> bool {
        false
    }
}

/// Our firmware's host link on USB: control transfers to its interface.
impl<U: Usb> Link for U {
    async fn hello(&self) -> Result<Vec<u8>> {
        let s = Setup { bm: VENDOR_IN, request: usb::HELLO, value: 0, index: self.iface() as u16 };
        self.control_in(s, crate::spec::MSG_MAX as u16, 2000).await.map_err(|e| {
            Error::Device(format!(
                "the device on {:04X}:{:04X} does not answer the host link ({e}): firmware from before the host \
                 link? DFU still works",
                usb::VID,
                usb::PID
            ))
        })
    }

    /// CALL stalls while an earlier request still runs.
    async fn call(&self, seq: u16, request: &[u8]) -> Result<bool> {
        let s = Setup { bm: VENDOR_OUT, request: usb::CALL, value: seq, index: self.iface() as u16 };
        match self.control_out(s, request, 2000).await {
            Ok(()) => Ok(true),
            Err(UsbError::Gone) => Err(Error::Usb("CALL", UsbError::Gone)),
            Err(_) => Ok(false),
        }
    }

    async fn poll(&self) -> Result<Vec<u8>> {
        let s = Setup { bm: VENDOR_IN, request: usb::POLL, value: 0, index: self.iface() as u16 };
        self.control_in(s, crate::spec::POLL_MAX as u16, 2000).await.map_err(|e| Error::Usb("POLL", e))
    }

    async fn sleep(&self, ms: u32) {
        Usb::sleep(self, ms).await
    }

    fn slow(&self) -> u32 {
        Usb::slow(self)
    }
}
