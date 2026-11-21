//! zd1-slot: the system slots (docs/module/storage.md; the library has how they work). What they hold: `zd1-ctl status`.
//!   zd1-slot mtd NAME           the MTD number of partition NAME (system-a, system-b, data, vendor), its nodes made
//!   zd1-slot install SLOT ID    a gzip'd cpio of a root on stdin into SLOT (a, b), stamped ID
//!   zd1-slot install-self SLOT ID [--start]   the system that runs from RAM (the rescue) into SLOT, stamped ID,
//!                               unless its stamp is ID already; --start: then the MCU starts SLOT once (zd1-linkd's
//!                               `try`)
//!   zd1-slot erase NAME         partition NAME (system-a, system-b, data) erased as from the factory, but for the
//!                               cleanmarkers (bad blocks left alone); refused while it is mounted. The vendor's: never
//! Which slot starts is the MCU's (`zd1 slot`).

use crate::{Slot, boot, erase, erase_slot, install, install_running, mtd, stamp};
use std::io::Write;
use std::os::unix::net::UnixStream;

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["mtd", name] => match mtd(name) {
            Some(n) => println!("{n}"),
            None => die(&format!("no partition \"{name}\"")),
        },
        ["install", s, id] => {
            let Some(s) = Slot::parse(s) else { die("SLOT: a or b") };
            match install(s, id, &mut std::io::stdin().lock(), &mut |_| Ok(())) {
                Ok(m) => println!("{m}"),
                Err(e) => die(&e),
            }
        }
        ["install-self", s, id, rest @ ..] if matches!(rest, [] | ["--start"]) => {
            let Some(s) = Slot::parse(s) else { die("SLOT: a or b") };
            if stamp(s).as_deref() == Some(*id) {
                println!("slot {}: holds {id} already", s.name());
            } else {
                match install_running(s, id) {
                    Ok(m) => println!("{m}"),
                    Err(e) => die(&e),
                }
            }
            if rest == ["--start"] {
                // the MCU starts it once: ready makes it the good slot (zd1-linkd: reg 0xD8)
                let line = format!("try {}\n", s.name());
                if let Err(e) = UnixStream::connect("/run/zd1link").and_then(|mut c| c.write_all(line.as_bytes())) {
                    die(&format!("/run/zd1link: {e}"));
                }
                println!("slot {}: asked the MCU to start it once", s.name());
            }
        }
        ["erase", name @ ("system-a" | "system-b" | "data")] => {
            let Some(n) = mtd(name) else { die(&format!("no partition \"{name}\"")) };
            // mounted (/data, or the running root: /dev/root in /proc/mounts): erasing it under the file system would
            // wreck what runs
            let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
            let root = boot().running.map(|s| s.partition()) == Some(*name);
            if root || mounts.lines().any(|l| l.split(' ').next() == Some(&format!("/dev/mtdblock{n}"))) {
                die(&format!("{name} (mtd{n}) is mounted: unmount it first (the rescue: `umount /data`)"));
            }
            let slot = [Slot::A, Slot::B].into_iter().find(|s| s.partition() == *name);
            match slot.map_or_else(|| erase(n), erase_slot) {
                Ok(bad) => println!("{name}: erased ({bad} bad blocks left alone)"),
                Err(e) => die(&e),
            }
        }
        _ => {
            eprintln!(
                "usage: zd1-slot mtd NAME | install SLOT ID < IMAGE | install-self SLOT ID [--start] | erase system-a|system-b|data"
            );
            std::process::exit(2);
        }
    }
}

fn die(e: &str) -> ! {
    zd1_sys::die("zd1-slot", e)
}
