//! zd1-app: the web page (host/web: https://pawlowskialex.github.io/zd1/, or the URL given) in a window of its own,
//! with what a browser makes the page ask for already there.
//!
//! The window is the system's web view (wry: WebKit on macOS and Linux, WebView2 on Windows). WebKit has no WebUSB,
//! and from an HTTPS page no web view opens plain `ws://` or `http://` to an address on the local network. So a
//! script goes into the page before its own (polyfill.js) and gives it:
//! - `navigator.usb` with the speaker's two devices, granted from the start: our firmware (03EB:2FE0) and the MCU's
//!   DFU bootloader (03EB:2FF8), through nusb here (usb.rs). Nothing else on the computer's USB is visible to it.
//! - a `WebSocket` and a `fetch` that reach private addresses, link-local ones, `.local` names and localhost here
//!   (net.rs): the speaker's link (docs/protocol/net.md, with the page's `Origin`), BL2's web updater.
//!
//! Only pages from the page's own origin, and localhost (`python3 tools/zeppelin web`), reach these; another page the
//! window is sent to opens in the system's browser instead.
//!
//! `ZD1_APP_LOG=1`: what the page asks for here and what the page logs, on stderr. `ZD1DFU_EMU=host:port`: the
//! emulator's USB bridge (`zemu run --mcu`, EP0 only) instead of the computer's USB, as for `zd1`.

// Windows: a window, no console beside it (debug builds keep theirs, for ZD1_APP_LOG)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod net;
mod usb;

use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

/// The published page.
const PAGE: &str = "https://pawlowskialex.github.io/zd1/";

/// What reaches the window's thread from the others: a script to run in the page, the page's title.
enum ToPage {
    Js(String),
    Title(String),
}

/// Where answers and events go: the page, through the window's thread.
#[derive(Clone)]
pub struct Page(EventLoopProxy<ToPage>);

impl Page {
    /// `ev` with its data to the polyfill's listeners (`__zd1.event`).
    pub fn event(&self, ev: &str, data: Value) {
        self.js(format!("__zd1.event({},{data})", json!(ev)));
    }

    fn js(&self, s: String) {
        let _ = self.0.send_event(ToPage::Js(s));
    }
}

/// One request's answer: the value, or a DOMException's name and message.
pub struct Reply {
    id: u64,
    page: Page,
}

pub type Answer = Result<Value, (&'static str, String)>;

impl Reply {
    pub fn send(self, a: Answer) {
        if let Err((name, msg)) = &a {
            log(&format!("#{} {name}: {msg}", self.id));
        }
        let (ok, v) = match a {
            Ok(v) => (true, v),
            Err((name, message)) => (false, json!({ "name": name, "message": message })),
        };
        self.page.js(format!("__zd1.done({},{ok},{v})", self.id));
    }
}

fn logging() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ZD1_APP_LOG").is_some_and(|v| !v.is_empty() && v != "0"))
}

pub fn log(s: &str) {
    if logging() {
        eprintln!("zd1-app: {s}");
    }
}

/// The origin (scheme://host[:port]) of a URL, lower-case; None for one without a host.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h); // no user info
    (!host.is_empty()).then(|| format!("{scheme}://{host}").to_ascii_lowercase())
}

/// Whether a page at `url` may use the bridge (and stay in the window): the page's origin, or this computer's.
fn trusted(url: &str, page: &str) -> bool {
    let Some(o) = origin(url) else { return false };
    if Some(&o) == origin(page).as_ref() {
        return true;
    }
    let Some(local) = o.strip_prefix("http://") else { return false };
    let host = local.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map_or(local, |(h, _)| h);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// A URL in the system's browser.
fn open_outside(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        eprintln!("zd1-app: can't open {url}: {e}");
    }
}

/// A request from the polyfill (`{id, op, ...}`) to the part that answers it.
fn dispatch(m: Value, page: &Page, usb: &Arc<usb::Usb>, net: &Arc<net::Net>, rt: &tokio::runtime::Handle) {
    let id = m["id"].as_u64().unwrap_or(0);
    let op = m["op"].as_str().unwrap_or("").to_string();
    if op == "log" {
        eprintln!("zd1-app: page: {}", m["text"].as_str().unwrap_or(""));
        return;
    }
    log(&format!("#{id} {op}"));
    let reply = Reply { id, page: page.clone() };
    match op.split_once('.') {
        Some(("usb", op)) => usb.request(op, m.clone(), reply, rt),
        Some(("net", op)) => net.request(op, m.clone(), reply, rt),
        _ => reply.send(Err(("NotSupportedError", format!("no {op} here")))),
    }
}

/// The macOS menu bar: the app menu (hide, quit), Edit, whose key equivalents (cut, copy, paste, select all, undo) are
/// how a text field in a web view gets them on macOS, and View (reload).
#[cfg(target_os = "macos")]
fn menu_bar() {
    use objc2::{MainThreadMarker, MainThreadOnly, sel};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let item = |title: &str, action: Option<objc2::runtime::Sel>, key: &str| unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    let menu = |title: &str, items: &[(&str, objc2::runtime::Sel, &str)]| {
        let m = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for (t, a, k) in items {
            m.addItem(&item(t, Some(*a), k));
        }
        let top = item(title, None, "");
        top.setSubmenu(Some(&m));
        top
    };
    let bar = NSMenu::new(mtm);
    bar.addItem(&menu("zd1", &[("Hide zd1", sel!(hide:), "h"), ("Quit zd1", sel!(terminate:), "q")]));
    bar.addItem(&menu(
        "Edit",
        &[
            ("Undo", sel!(undo:), "z"),
            ("Redo", sel!(redo:), "Z"),
            ("Cut", sel!(cut:), "x"),
            ("Copy", sel!(copy:), "c"),
            ("Paste", sel!(paste:), "v"),
            ("Select All", sel!(selectAll:), "a"),
        ],
    ));
    // the web view answers reload: itself (the responder chain); zoom is wry's hotkeys
    bar.addItem(&menu("View", &[("Reload", sel!(reload:), "r")]));
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}

fn main() -> wry::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| PAGE.to_string());
    if origin(&url).is_none() {
        eprintln!("usage: zd1-app [URL]   (the page: {PAGE})");
        std::process::exit(2);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;

    let event_loop = EventLoopBuilder::<ToPage>::with_user_event().build();
    let page = Page(event_loop.create_proxy());
    let usb = usb::Usb::new(page.clone(), rt.handle());
    let net = Arc::new(net::Net::new(page.clone()));

    let window = WindowBuilder::new()
        .with_title("zd1 · Zeppelin Air")
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 820.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(420.0, 480.0))
        .build(&event_loop)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    #[cfg(target_os = "macos")]
    menu_bar();

    // the page's storage (its identity for the network link, the speakers it knows, the MCU backups' copies): kept in
    // the user's data directory (macOS: WebKit's own store for this program)
    let mut context = WebContext::new(dirs::data_local_dir().map(|d| d.join("zd1-app")));
    let mut script = include_str!("polyfill.js").to_string();
    if logging() {
        script = script.replace("const LOG = false;", "const LOG = true;");
    }
    let (ipc_page, ipc_usb, ipc_net, ipc_rt, start) =
        (page.clone(), usb.clone(), net.clone(), rt.handle().clone(), url.clone());
    let (nav_start, title_page, load_usb, load_net) = (url.clone(), page.clone(), usb.clone(), net.clone());
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&url)
        .with_initialization_script_for_main_only(script, true)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            // the page that asks: the frame's URL (the main frame's on Linux)
            let from = req.uri().to_string();
            if !trusted(&from, &start) {
                log(&format!("refused a request from {from}"));
                return;
            }
            match serde_json::from_str::<Value>(req.body()) {
                Ok(mut m) => {
                    m["origin"] = json!(origin(&from)); // the WebSocket's Origin: the page's, not what it says
                    dispatch(m, &ipc_page, &ipc_usb, &ipc_net, &ipc_rt)
                }
                Err(e) => log(&format!("not a request: {e}")),
            }
        })
        .with_navigation_handler(move |to: String| {
            if trusted(&to, &nav_start)
                || to.starts_with("about:")
                || to.starts_with("blob:")
                || to.starts_with("data:")
            {
                return true;
            }
            open_outside(&to);
            false
        })
        .with_new_window_req_handler(|to, _| {
            open_outside(&to);
            NewWindowResponse::Deny
        })
        // the page's downloads (MCU backups, diagnostics): into Downloads, as a browser does
        .with_download_started_handler(|from, to| {
            log(&format!("download {from} -> {}", to.display()));
            true
        })
        .with_download_completed_handler(|_, to, ok| {
            if !ok {
                eprintln!("zd1-app: a download failed{}", to.map(|p| format!(": {}", p.display())).unwrap_or_default());
            }
        })
        // a page (re)loading lets go of what the last one had: its devices, its sockets
        .with_on_page_load_handler(move |ev, _| {
            if let wry::PageLoadEvent::Started = ev {
                load_usb.close_all();
                load_net.close_all();
            }
        })
        .with_document_title_changed_handler(move |t| {
            let _ = title_page.0.send_event(ToPage::Title(t));
        })
        .with_clipboard(true)
        .with_hotkeys_zoom(true)
        .with_devtools(cfg!(debug_assertions));
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("tao's GTK window has a box"))?
    };

    usb.watch();
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _ = &rt; // the runtime lives as long as the window
        match event {
            Event::UserEvent(ToPage::Js(s)) => {
                if let Err(e) = webview.evaluate_script(&s) {
                    log(&format!("evaluate: {e}"));
                }
            }
            Event::UserEvent(ToPage::Title(t)) => window.set_title(&t),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust() {
        assert!(trusted("https://pawlowskialex.github.io/zd1/#x", PAGE));
        assert!(trusted("https://PAWLOWSKIALEX.github.io/other", PAGE));
        assert!(!trusted("https://pawlowskialex.github.io.evil.example/zd1/", PAGE));
        assert!(!trusted("http://pawlowskialex.github.io/zd1/", PAGE));
        assert!(!trusted("https://x@evil.example/zd1/", PAGE));
        assert!(trusted("http://localhost:8000/", PAGE));
        assert!(trusted("http://127.0.0.1/", PAGE));
        assert!(!trusted("https://localhost.evil.example/", PAGE));
        assert!(!trusted("http://192.168.50.20/", PAGE));
        assert!(!trusted("about:blank", PAGE));
    }
}
