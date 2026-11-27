//! `zd1`'s HTTP to BL2's web updater (host/core's `bl2web::Http`): HTTP/1.1 over a socket of its own, one request per
//! connection, redirects followed as a browser follows them, everything readable. `--iface`: the socket bound to one
//! interface (a USB Ethernet dongle beside the Wi-Fi, both on 169.254/16: the computer may route BL2's 169.254.1.1
//! over the Wi-Fi, where it isn't).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use zd1_host::bl2web::{Http, Resp};
use zd1_host::{Error, Result};

pub struct Tcp {
    pub host: String,
    pub port: u16,
    pub iface: Option<String>,
}

impl Tcp {
    /// `HOST[:PORT]` (80 by default).
    pub fn new(target: &str, iface: Option<String>) -> Result<Tcp> {
        let (host, port) = match target.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().map_err(|_| Error::Failed(format!("{target}: not HOST[:PORT]")))?),
            None => (target, 80),
        };
        Ok(Tcp { host: host.into(), port, iface })
    }

    fn connect(&self, timeout: Duration) -> std::io::Result<TcpStream> {
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| std::io::Error::other(format!("{}: no IPv4 address", self.host)))?;
        let s = match &self.iface {
            None => TcpStream::connect_timeout(&addr, Duration::from_secs(4))?,
            Some(i) => bound(i, addr)?,
        };
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        Ok(s)
    }

    /// One request; (status, Location, body).
    fn once(
        &self,
        method: &str,
        path: &str,
        head: &str,
        body: &[u8],
        timeout: Duration,
    ) -> Result<(u16, Option<String>, String)> {
        let io = |e: std::io::Error| Error::Device(format!("{}:{}{path}: {e}", self.host, self.port));
        let mut s = self.connect(timeout).map_err(io)?;
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{head}Content-Length: {}\r\n\r\n",
            self.host,
            body.len()
        );
        s.write_all(req.as_bytes()).map_err(io)?;
        for (k, chunk) in body.chunks(64 * 1024).enumerate() {
            s.write_all(chunk).map_err(io)?;
            if body.len() > 256 * 1024 {
                eprint!("\r  sent {}/{} KB", ((k + 1) * 64 * 1024).min(body.len()) / 1024, body.len() / 1024);
            }
        }
        if body.len() > 256 * 1024 {
            eprintln!();
        }
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(io)?;
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| Error::Device(format!("{path}: no HTTP answer")))?;
        let head: String = raw[..split].iter().map(|&b| b as char).collect();
        let mut body = raw[split + 4..].to_vec();
        let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let header = |name: &str| {
            head.lines().skip(1).find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
            })
        };
        if header("Transfer-Encoding").is_some_and(|t| t.eq_ignore_ascii_case("chunked")) {
            body = unchunk(&body);
        }
        Ok((status, header("Location"), body.iter().map(|&b| b as char).collect()))
    }

    /// `method` and the redirects after it (a POST's as a GET, as a browser does).
    fn request(&self, method: &str, path: &str, head: &str, body: &[u8], timeout: Duration) -> Result<Resp> {
        let (mut status, mut loc, mut text) = self.once(method, path, head, body, timeout)?;
        let mut at = path.to_string();
        for _ in 0..5 {
            let Some(l) = loc.filter(|_| matches!(status, 301 | 302 | 303 | 307)) else { break };
            at = location_path(&l);
            (status, loc, text) = self.once("GET", &at, "", &[], Duration::from_secs(60))?;
        }
        Ok(Resp { status, path: at.split('?').next().unwrap_or("").to_string(), body: Some(text) })
    }
}

/// A Location's path (an absolute URL's, or the path itself).
fn location_path(l: &str) -> String {
    match l.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/".into(), |i| rest[i..].to_string()),
        None if l.starts_with('/') => l.into(),
        None => format!("/{l}"),
    }
}

/// A chunked body, joined.
fn unchunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(eol) = b[at..].windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&b[at..at + eol]).ok().and_then(|s| usize::from_str_radix(s.trim(), 16).ok());
        let Some(n) = size.filter(|&n| n > 0) else { break };
        let start = at + eol + 2;
        out.extend_from_slice(&b[start..(start + n).min(b.len())]);
        at = start + n + 2;
        if at >= b.len() {
            break;
        }
    }
    out
}

/// A socket bound to interface `iface`, connected to `addr` (macOS: IP_BOUND_IF; Linux: SO_BINDTODEVICE, which needs
/// CAP_NET_RAW or root before 5.7).
fn bound(iface: &str, addr: SocketAddr) -> std::io::Result<TcpStream> {
    use std::os::fd::FromRawFd;
    let SocketAddr::V4(v4) = addr else { return Err(std::io::Error::other("IPv4 only")) };
    let name = std::ffi::CString::new(iface).map_err(std::io::Error::other)?;
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let s = TcpStream::from_raw_fd(fd); // closed on every way out
        #[cfg(target_os = "macos")]
        let r = {
            let idx = libc::if_nametoindex(name.as_ptr());
            if idx == 0 {
                return Err(std::io::Error::other(format!("no interface {iface}")));
            }
            libc::setsockopt(fd, libc::IPPROTO_IP, libc::IP_BOUND_IF, (&idx as *const u32).cast(), 4)
        };
        #[cfg(not(target_os = "macos"))]
        let r = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast(),
            name.as_bytes().len() as libc::socklen_t,
        );
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sa = libc::sockaddr_in {
            #[cfg(target_os = "macos")]
            sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: v4.port().to_be(),
            sin_addr: libc::in_addr { s_addr: u32::from(*v4.ip()).to_be() },
            sin_zero: [0; 8],
        };
        let n = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        if libc::connect(fd, (&sa as *const libc::sockaddr_in).cast(), n) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(s)
    }
}

impl Http for Tcp {
    async fn get(&self, path: &str) -> Result<Resp> {
        self.request("GET", path, "", &[], Duration::from_secs(60))
    }

    async fn post_file(&self, path: &str, name: &str, data: &[u8], submit: &str) -> Result<Resp> {
        let boundary = format!("----zd1bl2web{}", zd1_host::seed());
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"appFirmware\"; filename=\"{name}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(
            format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"{submit}\"\r\n\r\nUpload\r\n--{boundary}--\r\n")
                .as_bytes(),
        );
        let head = format!("Content-Type: multipart/form-data; boundary={boundary}\r\n");
        self.request("POST", path, &head, &body, Duration::from_secs(900))
    }
}
