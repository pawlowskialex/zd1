//! A session's line to zd1-linkd (/run/zd1link, module/os/apps/zd1-linkd): the client's requests as `host SEQ HEX`,
//! answered `host SEQ HEX`; the State record as `state HEX` whenever it changes (`subscribe state`); `try SLOT` after an
//! update. Other lines (the receivers' events) are not ours.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub struct Linkd {
    r: BufReader<UnixStream>,
    w: UnixStream,
    line: String,
}

/// What came from zd1-linkd for us.
pub enum Line {
    Reply(u16, Vec<u8>),
    State(Vec<u8>),
}

impl Linkd {
    pub fn open() -> Result<Linkd, String> {
        let s = UnixStream::connect("/run/zd1link").map_err(|e| format!("/run/zd1link: {e}"))?;
        let w = s.try_clone().map_err(|e| e.to_string())?;
        let mut l = Linkd { r: BufReader::new(s), w, line: String::new() };
        l.send("subscribe state")?;
        Ok(l)
    }

    pub fn send(&mut self, line: &str) -> Result<(), String> {
        self.w.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("zd1-linkd: {e}"))
    }

    /// The next line of ours within `wait` (None: none came).
    pub fn next(&mut self, wait: Duration) -> Result<Option<Line>, String> {
        let end = Instant::now() + wait;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.r.get_ref().set_read_timeout(Some(left)).map_err(|e| e.to_string())?;
            match self.r.read_line(&mut self.line) {
                Ok(0) => return Err("zd1-linkd went away".into()),
                Ok(_) if self.line.ends_with('\n') => {
                    let l = std::mem::take(&mut self.line);
                    if let Some(x) = parse(l.trim_end()) {
                        return Ok(Some(x));
                    }
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("zd1-linkd: {e}")),
            }
        }
    }
}

fn parse(l: &str) -> Option<Line> {
    let (kind, rest) = l.split_once(' ')?;
    match kind {
        "host" => {
            let (seq, h) = rest.split_once(' ')?;
            Some(Line::Reply(seq.parse().ok()?, zd1_sys::unhex(h)?))
        }
        "state" => Some(Line::State(zd1_sys::unhex(rest)?)),
        _ => None,
    }
}
