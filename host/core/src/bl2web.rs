//! BL2's web updater (docs/module/boot-chain.md, "Web updater" and "The ZD1L BL2"): which BL2 answers at an address,
//! a file to it, its write followed. `zd1 bl2web`, `zd1 bl2-updater` and the page (its updater routes, the Lightning
//! model's install by address) drive it through [`Http`], whose answers may be unreadable: a browser fetches BL2 with
//! `no-cors` (BL2 sends no CORS headers), `zd1` and zd1-app read everything. Unreadable, the requests go blind, in the
//! order B&W's own pages send them, and nothing is checked.
//!
//! The two BL2s, told apart by `/1000/firmware_status.asp` (only the 2012 one has it):
//! - the Zeppelin Air's (2010): a `.bcd`, the module alone. POST it (`appFirmware`) to `/goform/formPostHandler`; BL2
//!   checks it (CRCs, HWID, version) and answers its confirm page; the confirm (`formDownloadConfirmationHandler`)
//!   starts the write; `bl_flash_burn_status.asp` says "Firmware upload status: N% completed", then "finished
//!   successfully" or "failed" (a status it keeps from the previous update until the new write ends), and BL2 resets
//!   the module after a whole write.
//! - the Lightning model's (2012): a B&W `.fw` (the module's `.bcd`, the MCU's `.dmb`). `/1000/firmware_prepare.asp`
//!   arms the upload; the POST redirects to `/1000/firmware_apply.asp`, whose (empty) answer arms the confirm, a
//!   non-empty one being a refusal (`_HWID`: other hardware); `/1000/firmware_expert.asp`, then the expert confirm
//!   (`formExpertDownloadConfirmationHandler`, which redirects to the status page; without the apply page's arming it
//!   goes to FWUCHandlerPage and writes nothing); `firmware_status.asp` until `<finished>YES`: the MCU part first (to
//!   the MCU's bootloader), then the module's.

use crate::{Error, Result};

/// An answer: its status, the path it ended at (redirects followed, as a browser follows them), its body. `body`
/// None: unreadable (a browser's `no-cors`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resp {
    pub status: u16,
    pub path: String,
    pub body: Option<String>,
}

/// HTTP to one BL2 (its address the implementation's): GET, and a file POSTed as BL2's form sends it (multipart:
/// the file as `appFirmware`, named `name`, then the form's submit field `submit` = "Upload"). An error when nothing
/// answers.
#[allow(async_fn_in_trait)]
pub trait Http {
    async fn get(&self, path: &str) -> Result<Resp>;
    async fn post_file(&self, path: &str, name: &str, data: &[u8], submit: &str) -> Result<Resp>;
}

/// What answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bl2 {
    /// the Lightning model's (2012), what it says is installed ("0": a part it hasn't asked about)
    Zd1l { model: String, module: String, mcu: String },
    /// the Zeppelin Air's (2010)
    Zd1,
    /// something that isn't BL2's web updater
    Other,
    /// something whose answers can't be read
    Blind,
}

/// The confirm buttons' value (BL2's aspSetFirmwareUploadConfirmationHandler), form-encoded.
const OK: &str = "++++++Ok++++++";

/// The text between `<name>` and `</name>` (BL2's XML status), trimmed; "" if there is none.
fn tag<'a>(xml: &'a str, name: &str) -> &'a str {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    xml.find(&open).map(|a| &xml[a + open.len()..]).and_then(|r| r.find(&close).map(|b| r[..b].trim())).unwrap_or("")
}

/// Every `<name>` value, in order.
fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        let r = &rest[a + open.len()..];
        let Some(b) = r.find(&close) else { break };
        out.push(r[..b].trim());
        rest = &r[b + close.len()..];
    }
    out
}

/// A page's text: tags out, whitespace folded.
pub fn text(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in html.trim_start_matches('\u{feff}').chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                out.push(' ');
            }
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Which BL2 answers.
pub async fn look<H: Http>(h: &H) -> Result<Bl2> {
    let r = h.get("/1000/firmware_status.asp").await?;
    let Some(body) = r.body else { return Ok(Bl2::Blind) };
    if r.status == 200 && body.contains("<Status>") {
        let cur = tag(&body, "current");
        return Ok(Bl2::Zd1l {
            model: tag(&body, "Model").into(),
            module: tag(cur, "version").into(),
            mcu: tag(cur, "host").into(),
        });
    }
    let old = h.get("/1000/bl_firmware_update.asp").await.ok().and_then(|r| r.body);
    Ok(if old.is_some_and(|b| b.contains("appFirmware")) { Bl2::Zd1 } else { Bl2::Other })
}

/// `data` (`name`: its file name) to the updater, written once confirmed: the Lightning model's sequence (`lightning`:
/// a `.fw`) or the Zeppelin Air's (a `.bcd`). Whether the answers were read (then the write started for sure: follow it
/// with [`burn`]); an error on a refusal it could read, before anything was written.
pub async fn upload<H: Http>(h: &H, lightning: bool, data: &[u8], name: &str) -> Result<bool> {
    let refused = |why: String| Error::Failed(format!("the updater didn't take the file: {why}; nothing was written"));
    if !lightning {
        let seen = h.get("/1000/bl_firmware_update.asp").await?.body.is_some();
        let up = h.post_file("/goform/formPostHandler", name, data, "appFirmware").await?;
        if let Some(body) = &up.body
            && !body.contains("formDownloadConfirmationHandler")
        {
            return Err(refused(text(body).chars().take(200).collect()));
        }
        h.get(&format!("/goform/formDownloadConfirmationHandler?button={OK}")).await?;
        return Ok(seen);
    }
    let seen = h.get("/1000/firmware_status.asp").await?.body.is_some();
    h.get("/1000/firmware_prepare.asp").await?;
    let up = h.post_file("/goform/formPostHandler", name, data, "appFirmwareFile").await?;
    if let Some(body) = &up.body {
        if up.path != "/1000/firmware_apply.asp" {
            return Err(refused(format!("it went to {}", up.path)));
        }
        let said = text(body);
        if said.contains("_HWID") {
            return Err(refused("it is for other hardware than this speaker's".into()));
        }
        if !said.is_empty() {
            return Err(refused(said.chars().take(200).collect()));
        }
    }
    h.get("/1000/firmware_expert.asp").await?;
    let c = h.get(&format!("/goform/formExpertDownloadConfirmationHandler?button={OK}")).await?;
    if c.body.is_some() && !c.path.ends_with("/firmware_status.asp") {
        return Err(Error::Failed(
            "the updater took the file but didn't start writing it: nothing was written; try again".into(),
        ));
    }
    Ok(seen)
}

/// Where a write stands: the part (the Lightning model's: "1" the main board's, "2" the module's; the Zeppelin Air's:
/// "") and its percent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub part: String,
    pub percent: Option<u32>,
}

/// The write [`upload`] started, followed on BL2's status page until it ends (`on` at each change): Ok once it
/// finished (the Zeppelin Air's: or stopped answering after 100 %, its reset), an error if it says it failed or stops
/// answering for long. Readable answers only.
pub async fn burn<H: Http>(h: &H, lightning: bool, mut on: impl FnMut(&Progress)) -> Result<()> {
    const POLL_MS: u32 = 2000;
    let (mut lost, mut last, mut failed) = (0u32, None::<Progress>, false);
    let page = if lightning { "/1000/firmware_status.asp" } else { "/1000/firmware_update/bl_flash_burn_status.asp" };
    loop {
        crate::sleep(POLL_MS).await;
        let body = match h.get(page).await.map(|r| r.body) {
            Ok(Some(b)) => {
                lost = 0;
                b
            }
            Ok(None) => return Err(Error::Failed("the updater's answers can't be read here".into())),
            Err(_) => {
                lost += 1;
                let whole = last.as_ref().is_some_and(|p| p.percent == Some(100));
                if !lightning && lost * POLL_MS >= 6000 && (whole || failed) {
                    return Ok(()); // it reset the module: the new image starts
                }
                if lost * POLL_MS >= 60_000 {
                    return Err(Error::Failed(
                        "the updater stopped answering during the write: give the speaker a few minutes (it starts \
                         what was written), then look at it"
                            .into(),
                    ));
                }
                continue;
            }
        };
        let now = if lightning {
            Progress {
                part: tag(&body, "module").into(),
                // the last part's that says a number (one not started yet says nothing)
                percent: tags(&body, "progress").iter().rev().find_map(|p| p.parse().ok()),
            }
        } else {
            let t = text(&body);
            let percent = t
                .split("Firmware upload status: ")
                .nth(1)
                .and_then(|r| r.split('%').next())
                .and_then(|n| n.trim().parse().ok());
            Progress { part: String::new(), percent }
        };
        if last.as_ref() != Some(&now) {
            on(&now);
            last = Some(now.clone());
        }
        if lightning {
            if tag(&body, "finished") == "YES" {
                let (module, mcu) = (tag(&body, "success"), tag(&body, "success1"));
                if module != "YES" || !matches!(mcu, "YES" | "") {
                    return Err(Error::Failed(format!(
                        "the updater says the write failed (the module's part {}, the main board's {}): the speaker \
                         stays in its updater; try again, or B&W's own file",
                        if module.is_empty() { "not done" } else { module },
                        if mcu.is_empty() { "not done" } else { mcu },
                    )));
                }
                return Ok(());
            }
        } else {
            let t = text(&body);
            if t.contains("finished successfully") && now.percent == Some(100) {
                return Ok(());
            }
            failed = t.contains("process failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A BL2 that answers from a table (path prefix -> (status, final path, body)), and records what was asked.
    struct Fake {
        pages: Vec<(&'static str, u16, &'static str, Option<&'static str>)>,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn answer(&self, path: &str) -> Result<Resp> {
            self.asked.borrow_mut().push(path.into());
            let (_, status, to, body) = self
                .pages
                .iter()
                .find(|(p, ..)| path.starts_with(p))
                .ok_or_else(|| Error::Failed(format!("no answer at {path}")))?;
            Ok(Resp { status: *status, path: to.to_string(), body: body.map(String::from) })
        }
    }

    impl Http for Fake {
        async fn get(&self, path: &str) -> Result<Resp> {
            self.answer(path)
        }
        async fn post_file(&self, path: &str, _name: &str, _data: &[u8], _submit: &str) -> Result<Resp> {
            self.answer(path)
        }
    }

    fn block<T>(f: impl std::future::Future<Output = T>) -> T {
        futures_lite::future::block_on(f)
    }

    #[test]
    fn tells_the_bl2s_apart() {
        let lightning = Fake {
            pages: vec![(
                "/1000/firmware_status.asp",
                200,
                "/1000/firmware_status.asp",
                Some(
                    "<Status><Model>Zeppelin Air</Model><current><version>105.9112</version><host>0</host></current></Status>",
                ),
            )],
            asked: RefCell::default(),
        };
        assert_eq!(
            block(look(&lightning)).unwrap(),
            Bl2::Zd1l { model: "Zeppelin Air".into(), module: "105.9112".into(), mcu: "0".into() }
        );
        let zd1 = Fake {
            pages: vec![
                ("/1000/firmware_status.asp", 404, "/1000/firmware_status.asp", Some("not found")),
                ("/1000/bl_firmware_update.asp", 200, "/1000/bl_firmware_update.asp", Some("<input name=appFirmware>")),
            ],
            asked: RefCell::default(),
        };
        assert_eq!(block(look(&zd1)).unwrap(), Bl2::Zd1);
        let blind = Fake { pages: vec![("/", 0, "", None)], asked: RefCell::default() };
        assert_eq!(block(look(&blind)).unwrap(), Bl2::Blind);
    }

    #[test]
    fn the_lightning_sequence_and_its_refusal() {
        let ok = Fake {
            pages: vec![
                ("/1000/firmware_status.asp", 200, "/1000/firmware_status.asp", Some("<Status/>")),
                ("/1000/firmware_prepare.asp", 200, "/1000/firmware_prepare.asp", Some("")),
                ("/goform/formPostHandler", 200, "/1000/firmware_apply.asp", Some("")),
                ("/1000/firmware_expert.asp", 200, "/1000/firmware_expert.asp", Some("")),
                ("/goform/formExpertDownloadConfirmationHandler", 200, "/1000/firmware_status.asp", Some("")),
            ],
            asked: RefCell::default(),
        };
        assert!(block(upload(&ok, true, b"fw", "zd1l.fw")).unwrap());
        assert_eq!(
            *ok.asked.borrow(),
            [
                "/1000/firmware_status.asp",
                "/1000/firmware_prepare.asp",
                "/goform/formPostHandler",
                "/1000/firmware_expert.asp",
                "/goform/formExpertDownloadConfirmationHandler?button=++++++Ok++++++",
            ]
        );
        let other_hw = Fake {
            pages: vec![
                ("/1000/firmware_status.asp", 200, "/1000/firmware_status.asp", Some("<Status/>")),
                ("/1000/firmware_prepare.asp", 200, "/1000/firmware_prepare.asp", Some("")),
                ("/goform/formPostHandler", 200, "/1000/firmware_apply.asp", Some("<b>_HWID</b>")),
            ],
            asked: RefCell::default(),
        };
        let e = block(upload(&other_hw, true, b"fw", "zd1l.fw")).unwrap_err().to_string();
        assert!(e.contains("other hardware"), "{e}");
        // blind: every request in order, nothing checked
        let blind = Fake { pages: vec![("/", 0, "", None)], asked: RefCell::default() };
        assert!(!block(upload(&blind, true, b"fw", "zd1l.fw")).unwrap());
        assert_eq!(blind.asked.borrow().len(), 5);
    }

    #[test]
    fn the_lightning_burn_reads_the_last_number() {
        let st = Fake {
            pages: vec![(
                "/1000/firmware_status.asp",
                200,
                "/1000/firmware_status.asp",
                Some(
                    "<Status><module>2</module><progress>100</progress><progress>57</progress><progress></progress><finished>YES</finished><success>YES</success><success1>YES</success1></Status>",
                ),
            )],
            asked: RefCell::default(),
        };
        let mut seen = Vec::new();
        block(burn(&st, true, |p| seen.push((p.part.clone(), p.percent)))).unwrap();
        assert_eq!(seen, [("2".to_string(), Some(57))]);
    }

    #[test]
    fn the_zeppelin_air_sequence_and_its_burn() {
        let up = Fake {
            pages: vec![
                ("/1000/bl_firmware_update.asp", 200, "/1000/bl_firmware_update.asp", Some("appFirmware")),
                (
                    "/goform/formPostHandler",
                    200,
                    "/1000/bl_confirm_download.asp",
                    Some("formDownloadConfirmationHandler"),
                ),
                (
                    "/goform/formDownloadConfirmationHandler",
                    200,
                    "/1000/firmware_update/bl_flash_burn_status.asp",
                    Some(""),
                ),
                (
                    "/1000/firmware_update/bl_flash_burn_status.asp",
                    200,
                    "/1000/firmware_update/bl_flash_burn_status.asp",
                    Some("Firmware upload status: 100% completed. The upload process finished successfully."),
                ),
            ],
            asked: RefCell::default(),
        };
        assert!(block(upload(&up, false, b"bcd", "os.bcd")).unwrap());
        let mut seen = Vec::new();
        block(burn(&up, false, |p| seen.push(p.percent))).unwrap();
        assert_eq!(seen, [Some(100)]);
        let refused = Fake {
            pages: vec![
                ("/1000/bl_firmware_update.asp", 200, "/1000/bl_firmware_update.asp", Some("appFirmware")),
                ("/goform/formPostHandler", 200, "/1000/bl_firmware_update.asp", Some("CRC check of image failed")),
            ],
            asked: RefCell::default(),
        };
        let e = block(upload(&refused, false, b"bcd", "os.bcd")).unwrap_err().to_string();
        assert!(e.contains("CRC check"), "{e}");
    }
}
