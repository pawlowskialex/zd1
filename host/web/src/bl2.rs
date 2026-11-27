//! host/core's BL2 web updater client (`bl2web`) over the page's `fetch`: zd1-app's reaches the local network and
//! reads every answer; a browser's, with `no-cors` (BL2 sends no CORS headers), answers opaque, which the client takes
//! as unreadable and goes blind (docs/protocol/net.md, The ZD1L's first install).

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use zd1_host::bl2web::{Http, Resp};
use zd1_host::{Error, Result};

/// One BL2: its address (`HOST[:PORT]`), the fetch mode ("no-cors" in the page: blind in a browser, read in zd1-app;
/// "cors" where every answer can be read, node's tests), a time limit for each request (0: none).
pub struct JsHttp {
    pub addr: String,
    pub mode: String,
    pub timeout_ms: u32,
}

fn js(e: JsValue) -> Error {
    let m = Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string()).or_else(|| e.as_string());
    Error::Device(m.unwrap_or_else(|| "fetch failed".into()))
}

fn global(name: &str) -> Result<JsValue> {
    Reflect::get(&js_sys::global(), &name.into()).map_err(js)
}

impl JsHttp {
    async fn fetch(&self, path: &str, init: Object) -> Result<Resp> {
        let url = format!("http://{}{path}", self.addr);
        let _ = Reflect::set(&init, &"mode".into(), &self.mode.as_str().into());
        let _ = Reflect::set(&init, &"cache".into(), &"no-store".into());
        if self.timeout_ms > 0
            && let Ok(t) =
                Reflect::get(&global("AbortSignal")?, &"timeout".into()).and_then(|t| t.dyn_into::<Function>())
        {
            let _ = Reflect::set(
                &init,
                &"signal".into(),
                &t.call1(&global("AbortSignal")?, &self.timeout_ms.into()).map_err(js)?,
            );
        }
        let f: Function = global("fetch")?.dyn_into().map_err(js)?;
        let p = f.call2(&js_sys::global(), &url.into(), &init).map_err(js)?;
        let r = JsFuture::from(js_sys::Promise::resolve(&p)).await.map_err(js)?;
        let get = |k: &str| Reflect::get(&r, &k.into()).unwrap_or(JsValue::UNDEFINED);
        if get("type").as_string().as_deref() == Some("opaque") {
            return Ok(Resp { status: 0, path: String::new(), body: None });
        }
        let status = get("status").as_f64().unwrap_or(0.0) as u16;
        let at = get("url").as_string().unwrap_or_default();
        let path = at.split_once("://").and_then(|(_, r)| r.find('/').map(|i| r[i..].to_string())).unwrap_or_default();
        let text: Function = get("text").dyn_into().map_err(js)?;
        let body = JsFuture::from(js_sys::Promise::resolve(&text.call0(&r).map_err(js)?)).await.map_err(js)?;
        Ok(Resp { status, path: path.split('?').next().unwrap_or("").into(), body: body.as_string() })
    }
}

impl Http for JsHttp {
    async fn get(&self, path: &str) -> Result<Resp> {
        self.fetch(path, Object::new()).await
    }

    async fn post_file(&self, path: &str, name: &str, data: &[u8], submit: &str) -> Result<Resp> {
        let parts = Array::of1(&Uint8Array::from(data));
        let ctor = |n: &str| -> Result<Function> { global(n)?.dyn_into::<Function>().map_err(js) };
        let blob = Reflect::construct(&ctor("Blob")?, &Array::of1(&parts)).map_err(js)?;
        let form = Reflect::construct(&ctor("FormData")?, &Array::new()).map_err(js)?;
        let append: Function = Reflect::get(&form, &"append".into()).map_err(js)?.dyn_into().map_err(js)?;
        append.call3(&form, &"appFirmware".into(), &blob, &name.into()).map_err(js)?;
        append.call2(&form, &submit.into(), &"Upload".into()).map_err(js)?;
        let init = Object::new();
        let _ = Reflect::set(&init, &"method".into(), &"POST".into());
        let _ = Reflect::set(&init, &"body".into(), &form);
        self.fetch(path, init).await
    }
}
