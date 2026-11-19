//! The speaker over the network from the page (docs/protocol/net.md): the browser's WebSocket to `ws://SPEAKER/zd1`
//! as host/core's [`Socket`]. From an HTTPS page the browser asks the person once to let it reach devices on the local
//! network (Local Network Access); a private address or a `.local` name needs nothing else.

use js_sys::{ArrayBuffer, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{BinaryType, MessageEvent, WebSocket};
use zd1_host::net::Socket;
use zd1_host::spec::net;
use zd1_host::{Error, Result};

pub struct WebSock {
    ws: WebSocket,
    inbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
    /// 0 opening, 1 open, 2 closed
    state: Rc<Cell<u8>>,
    _handlers: [Closure<dyn FnMut(JsValue)>; 3],
}

impl WebSock {
    /// The speaker's link at `addr` (HOST or HOST:PORT), open.
    pub async fn open(addr: &str) -> Result<WebSock> {
        let url = format!("ws://{addr}{}", net::PATH);
        let ws = WebSocket::new(&url).map_err(|e| Error::Device(format!("{url}: {e:?}")))?;
        ws.set_binary_type(BinaryType::Arraybuffer);
        let (inbox, state) = (Rc::new(RefCell::new(VecDeque::new())), Rc::new(Cell::new(0u8)));
        let i = inbox.clone();
        let on_message = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            if let Ok(b) = e.unchecked_into::<MessageEvent>().data().dyn_into::<ArrayBuffer>() {
                i.borrow_mut().push_back(Uint8Array::new(&b).to_vec());
            }
        });
        let (s1, s2) = (state.clone(), state.clone());
        let on_open = Closure::<dyn FnMut(JsValue)>::new(move |_| s1.set(1));
        let on_close = Closure::<dyn FnMut(JsValue)>::new(move |_| s2.set(2));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let s = WebSock { ws, inbox, state, _handlers: [on_message, on_open, on_close] };
        let t0 = zd1_host::now_ms();
        while s.state.get() == 0 && zd1_host::now_ms() - t0 < 10_000.0 {
            zd1_host::sleep(20).await;
        }
        if s.state.get() != 1 {
            return Err(Error::Device(format!(
                "no answer at {url}: is the speaker on this network and running our software, and may this page \
                 reach devices on the local network (the browser asks once)?"
            )));
        }
        Ok(s)
    }
}

impl Socket for WebSock {
    async fn send(&self, data: &[u8]) -> Result<()> {
        if self.state.get() != 1 {
            return Err(Error::Device("the connection to the speaker has closed".into()));
        }
        self.ws.send_with_u8_array(data).map_err(|e| Error::Device(format!("the connection to the speaker: {e:?}")))
    }

    async fn recv(&self, ms: u32) -> Result<Option<Vec<u8>>> {
        let t0 = zd1_host::now_ms();
        loop {
            if let Some(m) = self.inbox.borrow_mut().pop_front() {
                return Ok(Some(m));
            }
            if self.state.get() == 2 {
                return Err(Error::Device("the speaker closed the connection".into()));
            }
            if zd1_host::now_ms() - t0 >= ms as f64 {
                return Ok(None);
            }
            zd1_host::sleep(10).await;
        }
    }

    async fn sleep(&self, ms: u32) {
        zd1_host::sleep(ms).await
    }
}

impl Drop for WebSock {
    fn drop(&mut self) {
        self.ws.set_onmessage(None);
        self.ws.set_onopen(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}
