//! The system's services and early start (the full build only; the rescue's build leaves this file out, so it
//! doesn't change when a system service does). The rescue's: services.rs, start.rs.

use super::say;
use super::services::{
    Backoff, Check, Command, Ctx, Health, Lifetime, Output, Service, Start, command, daemon, run, shell,
};
use super::start::{Boot, data};
use super::supervisor::PATH;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, Instant};
use zd1_net::wpa::{Stall, Watch};
use zd1_net::{Iface, netmask, parse_cidr, set_name_servers};
use zd1_settings::NetMode;
use zd1_sys::{OsRelease, cstr, write_private};

pub const SERVICES: &[Service] = &[
    Service {
        name: "entropy",
        about: "credits the kernel's entropy pool (it reads 0 here and 2.6.24 has no getrandom)",
        start: entropy,
        stop: None,
        lifetime: Lifetime::Once { wait: true, retry: None },
        output: Output::Log,
        settings: &[],
        health: None,
    },
    Service {
        name: "zd1-linkd",
        about: "the MCU link (module/link over the SPI slave)",
        start: |_| run(&["zd1-linkd"]),
        stop: None,
        lifetime: daemon(1000),
        output: Output::LogAndKernel,
        settings: &[],
        health: None,
    },
    Service {
        name: "ethernet",
        about: "eth0: the PHY, then off, DHCP (link-local 169.254.1.1 until a lease) or the fixed address",
        start: ethernet,
        stop: Some(ethernet_down),
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &["eth."],
        health: None,
    },
    Service {
        name: "wifi",
        about: "wlan0: wpa_supplicant (the settings' network if any, else idle for scans), DHCP or a fixed address",
        start: wifi,
        stop: Some(wifi_down),
        lifetime: daemon(5000),
        output: Output::Log,
        settings: &["wifi."],
        health: Some(Health { every: Duration::from_secs(10), new: wifi_health }),
    },
    Service {
        name: "time",
        about: "the wall clock from NTP, once (TLS needs it; the module has no clock that runs off)",
        start: |_| run(&["ntpd", "-n", "-q", "-p", "pool.ntp.org"]),
        stop: None,
        lifetime: Lifetime::Once {
            wait: false,
            retry: Some(Backoff { first: Duration::from_secs(5), times: 12, then: Duration::from_secs(60) }),
        },
        output: Output::Log,
        settings: &[],
        health: None,
    },
    Service {
        name: "net",
        about: "the network link (zd1-netd): paired clients reach the speaker on port 80 (docs/protocol/net.md)",
        start: |c| {
            if c.setting("net") == "off" {
                return Start::Off("off (the setting net)".into());
            }
            run(&["zd1-netd"])
        },
        stop: None,
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &["net"],
        health: None,
    },
    Service {
        name: "snapcast",
        about: "the Snapcast receiver (zd1-snapcast)",
        start: snapcast,
        stop: None,
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &["name", "snapcast", "snapcast."],
        health: None,
    },
    Service {
        name: "airplay",
        about: "the AirPlay 1/2 receiver (zd1-airplay)",
        start: airplay,
        stop: None,
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &["name", "airplay", "airplay."],
        health: None,
    },
    Service {
        name: "spotify",
        about: "the Spotify Connect receiver (zd1-spotify)",
        start: spotify,
        stop: None,
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &["name", "spotify", "spotify."],
        health: None,
    },
    Service {
        name: "debug",
        about: "a root shell on telnet 2323 and zd1-memsh on 23; only the MCU switches them on",
        start: debug,
        stop: None,
        lifetime: daemon(2000),
        output: Output::Log,
        settings: &[],
        health: None,
    },
    Service {
        name: "self-install",
        about: "the rescue copies itself into a slot: an empty one, or when asked (zd1.install), then the MCU starts it",
        start: self_install,
        stop: None,
        lifetime: Lifetime::Once { wait: false, retry: None },
        output: Output::LogAndKernel,
        settings: &[],
        health: None,
    },
    Service {
        name: "selftest",
        about: "the userspace self test (zd1.selftest on the command line)",
        start: selftest,
        stop: None,
        lifetime: Lifetime::Once { wait: true, retry: None },
        output: Output::Console,
        settings: &[],
        health: None,
    },
    Service {
        name: "console",
        about: "a shell on the console",
        start: shell,
        stop: None,
        lifetime: daemon(1000),
        output: Output::Shell,
        settings: &[],
        health: None,
    },
];

/// Where `program` is, on init's PATH (the system has the core programs in /bin, the rest in /usr/bin); None: not in
/// this image.
fn installed(program: &str) -> Option<String> {
    PATH.split(':').map(|d| format!("{d}/{program}")).find(|p| Path::new(p).exists())
}

/// Off by a command-line switch or an `off` setting (the receivers'), and why.
fn switched_off(c: &Ctx, switch: &str, setting: &str) -> Option<String> {
    if c.boot.switch(switch) {
        return Some(format!("off ({switch} on the kernel command line)"));
    }
    if c.setting(setting) == "off" {
        return Some(format!("off (the setting {setting})"));
    }
    None
}

// ---- the services' starts ----

/// The kernel's entropy pool reads 0 on this board (no driver feeds it): anything that waits for /dev/random (the
/// AirPlay pairing keys, libsodium) would wait forever. zd1-entropy credits it from CPU timing jitter, ~0.2 s
/// (docs/streaming.md).
/// The rescue is the release's whole system (the boot image's initramfs, docs/module/storage.md). Booted as the rescue it
/// copies itself into a slot: when the slot an update would take holds no system (a first install: the next start is
/// that slot's), or when asked (`zd1.install`: the host's `Boot INSTALL` after a boot image, so that its system needs
/// no second upload) into the slot that isn't the good one (the good one if that holds none), which the MCU then
/// starts once; a slot holding this version already is started without a copy.
fn self_install(c: &Ctx) -> Start {
    let b = zd1_slot::boot();
    if !c.boot.switch("zd1.rescue") || b.running.is_some() {
        return Start::Off("not the full system booted as the rescue".into());
    }
    let id = zd1_sys::OsRelease::read("/etc/os-release").get("VERSION_ID").to_string();
    if c.boot.switch("zd1.install") {
        let good = b.target();
        let holds = |s: zd1_slot::Slot| zd1_slot::stamp(s).is_some_and(|st| st.split(' ').next() == Some(&id));
        let to = match [good.other(), good].into_iter().find(|&s| holds(s)) {
            Some(s) => s,
            None if zd1_slot::stamp(good).is_none() => good,
            None => good.other(),
        };
        return run(&["zd1-slot", "install-self", to.name(), &id, "--start"]);
    }
    let to = b.target();
    if zd1_slot::stamp(to).is_some() {
        return Start::Off(format!("slot {} holds a system", to.name()));
    }
    run(&["zd1-slot", "install-self", to.name(), &id])
}

fn entropy(_: &Ctx) -> Start {
    match installed("zd1-entropy") {
        Some(p) => run(&[&p]),
        None => Start::Off("not in this image".into()),
    }
}

/// eth0. The PHY is on the product board: it answers once the board is up (the pads, its clock, its reset released)
/// and a moment has passed. The driver probes it at every `up`: until it answers, again every 10 s, each attempt on
/// the kernel log (readable over JTAG: docs/module/jtag.md). Then link-local 169.254.1.1 at once (the Mac's direct
/// link has no DHCP server), and udhcpc, whose lease replaces it (zd1-dhcp).
fn ethernet(c: &Ctx) -> Start {
    const PHY_OUT_OF_RESET: Duration = Duration::from_millis(50);
    const ATTEMPTS: u32 = 90;
    if !Iface::exists("eth0") {
        return Start::Off("no eth0".into());
    }
    let since = c.boot.board_up.elapsed();
    if since < PHY_OUT_OF_RESET {
        return Start::Later(PHY_OUT_OF_RESET - since, "the PHY comes out of reset".into());
    }
    let eth = match Iface::open("eth0") {
        Ok(e) => e,
        Err(e) => return Start::Off(format!("eth0: {e}")),
    };
    if let Err(e) = eth.set_mac(unit_mac()) {
        say(&format!("ethernet: MAC: {e}"));
    }
    match eth.up() {
        Err(e) if c.attempt < ATTEMPTS => {
            Start::Later(Duration::from_secs(10), format!("the PHY doesn't answer ({e})"))
        }
        Err(e) => Start::Off(format!("the PHY never answered: {e}")),
        Ok(()) => {
            let a = zd1_settings::address(&c.settings, "eth");
            if a.mode == NetMode::Off {
                let _ = eth.down();
                return Start::Off("off (the setting eth.mode)".into());
            }
            let _ = eth.set_ipv4(Ipv4Addr::new(169, 254, 1, 1), netmask(16)); // the direct link's address until a lease
            match address(&eth, &a, "20") {
                Ok(p) if p.is_empty() => Start::Off(format!("fixed address {}", a.address)),
                Ok(p) => Start::Run(p),
                Err(e) => Start::Off(e),
            }
        }
    }
}

fn ethernet_down() {
    if let Ok(e) = Iface::open("eth0") {
        e.remove_default_routes();
        let _ = e.set_ipv4(Ipv4Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED);
    }
}

/// An interface's address from its settings: DHCP (udhcpc, a process that stays), or the fixed address, gateway
/// and name server (set here, no process). `retry_s`: udhcpc's pause between rounds of requests.
fn address(i: &Iface, a: &zd1_settings::Address, retry_s: &str) -> Result<Vec<Command>, String> {
    if a.mode == NetMode::Dhcp {
        return Ok(vec![command(&["udhcpc", "-i", i.name(), "-f", "-t", "5", "-A", retry_s, "-s", "/bin/zd1-dhcp"])]);
    }
    let Some((addr, mask)) = parse_cidr(&a.address) else {
        return Err(format!(
            "{}: static without an address ({}.address)",
            i.name(),
            if i.name() == "eth0" { "eth" } else { "wifi" }
        ));
    };
    let _ = i.set_ipv4(addr, mask);
    let gw = a.gateway.parse::<Ipv4Addr>().ok();
    if let Some(gw) = gw {
        i.remove_default_routes();
        let _ = i.add_default_route(gw);
    }
    if let Ok(dns) = a.dns.parse::<Ipv4Addr>() {
        let _ = set_name_servers(&[dns]);
    }
    say(&format!(
        "{}: fixed address {}{}",
        i.name(),
        a.address,
        gw.map(|g| format!(" router {g}")).unwrap_or_default()
    ));
    Ok(Vec::new())
}

/// eth0's MAC: sulu_eth comes up with 0 (Nokia's userspace set it). The unit's from PersParam, else a locally
/// administered one (the same on every unit: only for a broken PersParam).
fn unit_mac() -> [u8; 6] {
    zd1_persparam::unit_mac().unwrap_or([0x02, 0x5A, 0xD1, 0x00, 0x00, 0x01])
}

const WPA_CONF: &str = "/run/wpa_supplicant.conf";

/// Wi-Fi on wlan0 (sulu_wlan): wpa_supplicant over Wireless Extensions, running whenever the radio is on (the
/// setting wifi.mode), so that zd1-linkd can read its status and ask it for a scan (zd1-net's `wpa`); with known
/// networks (`zd1 wifi`) it joins the preferred one it sees, and the address comes by DHCP or is the settings' fixed
/// one. WPA3-only networks can't be joined this way (WEXT has no SAE).
fn wifi(c: &Ctx) -> Start {
    if !Iface::exists("wlan0") {
        return Start::Off("no wlan0 (sulu_wlan not loaded)".into());
    }
    if c.boot.switch("zd1.nowifi") {
        return Start::Off("off (zd1.nowifi on the kernel command line)".into());
    }
    let a = zd1_settings::address(&c.settings, "wifi");
    if a.mode == NetMode::Off {
        return Start::Off("off (the setting wifi.mode)".into());
    }
    let nets = zd1_settings::networks();
    if let Err(e) = write_private(WPA_CONF, &zd1_settings::wpa_conf(&nets)) {
        return Start::Off(format!("{WPA_CONF}: {e}"));
    }
    let wlan = match Iface::open("wlan0") {
        Ok(w) => w,
        Err(e) => return Start::Off(format!("wlan0: {e}")),
    };
    if let Err(e) = wlan.up() {
        return Start::Later(Duration::from_secs(5), format!("wlan0 up: {e}"));
    }
    let mut procs = vec![command(&["wpa_supplicant", "-Dwext", "-iwlan0", &format!("-c{WPA_CONF}")])];
    if nets.is_empty() {
        say("wifi: no network (zd1 wifi): the radio scans when asked");
        return Start::Run(procs);
    }
    match address(&wlan, &a, "10") {
        Ok(p) => procs.extend(p),
        Err(e) => return Start::Off(e),
    }
    Start::Run(procs)
}

/// The Wi-Fi's health: a known network joined, or the radio started again (zd1-net's `wpa::Stall`: after 5 min
/// unjoined, then ever later). The supplicant's status as zd1-linkd reads it, a few ms (at most 300 ms a request).
struct WifiHealth {
    watch: Watch,
    stall: Stall,
}

fn wifi_health() -> Box<dyn Check> {
    Box::new(WifiHealth { watch: Watch::new("wlan0"), stall: Stall::default() })
}

impl Check for WifiHealth {
    fn started(&mut self) {
        self.stall.started();
    }

    fn check(&mut self, now: Instant) -> Option<String> {
        self.watch.poll();
        let unjoined = self.stall.check(now, self.watch.state)?;
        Some(format!("no network joined for {} min", unjoined.as_secs() / 60))
    }
}

fn wifi_down() {
    let _ = std::fs::remove_file(WPA_CONF);
    if let Ok(w) = Iface::open("wlan0") {
        w.remove_default_routes();
        let _ = w.set_ipv4(Ipv4Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED);
        let _ = w.down();
    }
}

/// Snapcast (apps/zd1-snapcast): finds a snapserver over mDNS, or the settings' (HOST or HOST:PORT). More options in
/// /etc/snapcast.conf, a line `SNAPCAST_OPTS="..."`.
fn snapcast(c: &Ctx) -> Start {
    let Some(p) = installed("zd1-snapcast") else { return Start::Off("not in this image".into()) };
    if let Some(why) = switched_off(c, "zd1.nosnapcast", "snapcast") {
        return Start::Off(why);
    }
    let mut argv = vec![p, "--name".into(), c.setting("name")];
    match c.setting("snapcast.latency").as_str() {
        "" | "0" => {}
        ms => argv.extend(["--latency".into(), ms.into()]),
    }
    match c.setting("snapcast.server").as_str() {
        "" => {}
        s => match s.rsplit_once(':') {
            Some((host, port)) => argv.extend(["--host".into(), host.into(), "--port".into(), port.into()]),
            None => argv.extend(["--host".into(), s.into()]),
        },
    }
    let conf = std::fs::read_to_string("/etc/snapcast.conf").unwrap_or_default();
    if let Some(opts) = conf.lines().find_map(|l| l.trim().strip_prefix("SNAPCAST_OPTS=")) {
        argv.extend(opts.trim_matches('"').split_whitespace().map(String::from));
    }
    Start::Run(vec![Command { argv, arg0: None }])
}

/// A receiver named by the settings (`name`: "Zeppelin Air XXXXXX" from the MAC unless chosen), with its option
/// (`--flag`, the setting `option`) when that is set.
fn receiver(c: &Ctx, program: &str, switch: &str, setting: &str, option: (&str, &str)) -> Start {
    let Some(p) = installed(program) else { return Start::Off("not in this image".into()) };
    if let Some(why) = switched_off(c, switch, setting) {
        return Start::Off(why);
    }
    let mut argv = vec![p.as_str(), "--name"];
    let name = c.setting("name");
    argv.push(&name);
    let v = c.setting(option.1);
    if !v.is_empty() {
        argv.extend([option.0, v.as_str()]);
    }
    run(&argv)
}

/// AirPlay: with `airplay.pin`, pairing takes that PIN and the senders are remembered (zd1-airplay's store).
fn airplay(c: &Ctx) -> Start {
    receiver(c, "zd1-airplay", "zd1.noairplay", "airplay", ("--pin", "airplay.pin"))
}

fn spotify(c: &Ctx) -> Start {
    receiver(c, "zd1-spotify", "zd1.nospotify", "spotify", ("--bitrate", "spotify.bitrate"))
}

/// A login-less root shell and, in the debug system, raw memory for anyone on the network: off unless the MCU switches
/// them on (`zd1 debug on`, only from the speaker's USB port); without its word, the image decides (zd1.debug:
/// os-debug.bcd and the emulator's dev boot).
fn debug(c: &Ctx) -> Start {
    let on = c.debug.unwrap_or_else(|| c.boot.switch("zd1.debug"));
    if !on {
        return Start::Off("off (zd1 debug on)".into());
    }
    let mut procs = vec![command(&["telnetd", "-F", "-p", "2323", "-l", "/bin/sh"])];
    if let Some(memsh) = installed("zd1-memsh") {
        procs.push(command(&[&memsh, "23"]));
    }
    Start::Run(procs)
}

fn selftest(c: &Ctx) -> Start {
    if !c.boot.switch("zd1.selftest") {
        return Start::Off("off (zd1.selftest on the kernel command line)".into());
    }
    match installed("zd1-selftest") {
        Some(p) => run(&[&p]),
        None => Start::Off("not in this image".into()),
    }
}

// ---- the early start: the system's part (the rescue's: start.rs) ----

/// After the mounts: the host name, the identification, /data (formatted if it isn't ours yet), the vendor's
/// modules, loopback.
pub fn early(boot: &Boot, kernel: &str) {
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        unsafe {
            libc::sethostname(h.as_ptr() as *const _, h.len());
        }
    }
    let release = identify(boot);
    say(&format!(
        "{} {} ({}, {}), kernel {kernel}, {}",
        release.get("NAME"),
        release.get("VERSION_ID"),
        release.get("VARIANT_ID"),
        release.get("BUILD_DATE"),
        where_we_run(boot)
    ));
    data();
    modules();
    loopback();
}

/// The system's identification, with how it started as its variant (the command line: `zd1.rescue`, the system from
/// RAM as the rescue, which tells the MCU, `zd1-linkd`'s "zd1-os VERSION rescue", and the page that no slot holds what
/// runs; `zd1.debug`, the debug image or the emulator's dev boot; else release), as /run/os-release (/etc/os-release
/// points there: / is read-only).
fn identify(boot: &Boot) -> OsRelease {
    let base = std::fs::read_to_string("/usr/lib/os-release").unwrap_or_default();
    let variant = if boot.switch("zd1.rescue") {
        "rescue"
    } else if boot.switch("zd1.debug") {
        "debug"
    } else {
        "release"
    };
    let r = OsRelease::read("/usr/lib/os-release");
    let pretty = format!("{} {} ({variant}, {})", r.get("NAME"), r.get("VERSION_ID"), r.get("BUILD_DATE"));
    let _ = std::fs::write("/run/os-release", format!("{base}VARIANT_ID={variant}\nPRETTY_NAME=\"{pretty}\"\n"));
    OsRelease::read("/run/os-release")
}

/// Which slot runs (the boot stub put zd1.slot and zd1.good on the command line; docs/module/storage.md).
fn where_we_run(boot: &Boot) -> String {
    match (boot.arg("zd1.slot"), boot.arg("zd1.good")) {
        (Some(s), Some(g)) if s != g => format!("slot {s}, on trial (good: {g})"),
        (Some(s), _) => format!("slot {s}"),
        (None, _) => "no slot (an initramfs)".into(),
    }
}

/// The vendor's kernel modules, in /etc/modules' order (os.toml [vendor] load). sulu_wlan gets the unit's Wi-Fi MAC
/// (PersParam MAC0, locally administered: its own address) and `diversity=2`: the Zeppelin's antenna is on the
/// radio's second port. The driver's default (both) transmits on the first: the AP heard the module 15-20 dB weaker
/// than the module heard it, and ~1-2 % of the uplink was lost; on port 2 the link is symmetric and loses nothing
/// [HW 2026-10-24, docs/module/memory-map.md, Wi-Fi].
fn modules() {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let dir = format!("/lib/modules/{}/kernel", release.trim());
    let wlan_mac = match zd1_persparam::unit_mac() {
        Ok(m) => Some(zd1_persparam::wlan_mac(&m).iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(",")),
        Err(e) => {
            say(&format!("no MAC in PersParam: {e}"));
            None
        }
    };
    for m in std::fs::read_to_string("/etc/modules").unwrap_or_default().lines().filter(|l| !l.is_empty()) {
        let params = match m.rsplit('/').next() {
            Some("sulu_wlan") => {
                format!("diversity=2{}", wlan_mac.as_ref().map(|w| format!(" macaddr={w}")).unwrap_or_default())
            }
            _ => String::new(),
        };
        if let Err(e) = insmod(&format!("{dir}/{m}.ko"), &params) {
            say(&format!("insmod {m}: {e}"));
        }
    }
}

fn insmod(path: &str, params: &str) -> Result<(), String> {
    let image = std::fs::read(path).map_err(|e| e.to_string())?;
    let r = unsafe { libc::syscall(libc::SYS_init_module, image.as_ptr(), image.len(), cstr(params).as_ptr()) };
    if r != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

/// lo, 127.0.0.1: local TCP (the self test, the receivers' own control ports).
fn loopback() {
    let up = zd1_net::Iface::open("lo").and_then(|lo| {
        lo.set_ipv4(std::net::Ipv4Addr::LOCALHOST, zd1_net::netmask(8))?;
        lo.up()
    });
    if let Err(e) = up {
        say(&format!("lo: {e}"));
    }
}
