# Storage: the module OS on the NAND

How our module OS keeps its programs and its settings in the module's NAND, so only what it needs to start lives in
RAM. Tags: `docs/README.md`. The NAND itself (layout, ECC, the vendor's partitions): `boot-chain.md`.

## Partitions
The kernel command line (`module/os/os.toml`) cuts the chip into four MTD partitions (`mtdparts=sulu-nand:...`, the
Nokia HD-1's own syntax for this driver, `re/netmodule/NOKIA_HD1.md`):

| | NAND | Name | What |
|---|---|---|---|
| mtd0 | 0 - 32 MB | `vendor`, **read-only** | BL1, PersParam, CnE, the user settings, BL2, the app partition (our boot stub, kernel and the system's copy, the rescue), resources: the vendor's addresses are logical (bad blocks skipped), so the partition leaves 16 MB for bad blocks behind them; also 0x1300000, the stock app's `CnET` |
| mtd1 | 32 - 64 MB | `system-a` | system slot A (JFFS2: a whole root, the kernel's root file system, read-only) |
| mtd2 | 64 - 96 MB | `system-b` | system slot B |
| mtd3 | 96 - 128 MB | `data` | our data (JFFS2, read-write at `/data`): the settings (`settings.md`) |

The vendor partition is read-only to the kernel: our OS can't write BL1, BL2 or PersParam even by mistake. It is read
raw (`zd1-persparam`): the kernel's soft ECC miscorrects the vendor's pages (`boot-chain.md`, NAND).
The slots and `data` are Linux's own: written through MTD with the kernel's ECC (spare bytes 40-63) and JFFS2's
cleanmarkers (spare bytes 2-9), read back the same way, as on the Nokia HD-1, whose rootfs was JFFS2 on this driver.
Nothing of the vendor's chain reads them: BL2's updater erases and writes only its own partitions.

## What is where

| | Size (gzip), and the room | |
|---|---|---|
| app partition: the boot stub, the kernel (deflated: the stub inflates it), the rescue | 6.6 MB: kernel 1.40, rescue 5.1, stub 0.02 (`os.bcd` and `os-zd1l.bcd` 6.6 MB with B&W's preloader and the model's CnE, no resources); the vendor's app slot is 10 MB (0x240000-0xC3FFFF, `boot-chain.md`) | the rescue: the release system as an initramfs (`/init` its init), 10 MB in RAM while it runs, which it does only when no slot gets ready |
| a system slot (`/`) | 5.1 MB (10.3 MB unpacked) of the slot's 32 MB | busybox (106 applets, `os.toml [busybox] applets`), the full `zd1` (`/usr/bin/zd1`: init and the core programs in `/bin`, the receivers, the self test, the Warp and memory probes in `/usr/bin`), `/etc` (`module/os/rootfs`), Nokia's kernel modules and Wi-Fi firmware, `wpa_supplicant`, ALSA's configuration and tools |
| data partition (`/data`) | | `/data/zd1/settings` |

The running system holds no file system in RAM: the kernel mounts the slot as its root, its files are read from the
NAND into the page cache as they are used, and `/dev`, `/run`, `/tmp`, `/var` are tmpfs (init), each with a ceiling
(64 KB, 512 KB, 8 MB, 1 MB) so that no writer can take the receivers' memory; init keeps each service's log in
`/var/log` under 64 KB (`module/os/README.md`). `/etc/os-release`
(init: the system's `/usr/lib/os-release` plus the variant from the command line) and `/etc/resolv.conf` point into
`/run`. [HW 2026-10-26: 4.1 MB used, 24.3 MB available with every service running; the emulator said the same]

## The boot: which slot
The app partition holds the boot stub, the kernel and the rescue (the system, as of the image's install), and changes
rarely (BL2 or the relay write it).
Updating the OS means writing the slot that doesn't run, from the running system (`zd1-slot install`;
`docs/module/updater.md`), and starting it once on trial. **Which slot starts is the MCU's choice**
(`mcu/src/modboot.c`), kept in its EEPROM (the settings block's flags: the good slot, a slot to try once):
1. **The stub asks**, if our MCU knocked. The stub keeps the link muted (it takes requests and answers none) until
   our MCU writes reg 0xC4 = 0xFFFF, which it does every 200 ms while it waits for a module it powered (our preloader
   too); no knock within 3 s: the rescue, without a word on the link. B&W's bootloaders never knock, and the ZD1L's
   gives up on a module that answered and then went quiet for the kernel's boot (it waits for a key press from then
   on): with the stub answering, it never got to the update [EMU 2026-11-19, `docs/mcu/bootloader-hbl.md`].
   After the knock the stub says `zd1-boot` on reg 0x25 with status booting
   (0x2000) and waits up to 5 s for the MCU's answer on reg 0xC4 (`docs/protocol/registers.md`): b0 slot A, slot B
   or the rescue, b1 the good slot. A slot: the stub adds `root=/dev/mtdblock1` (or 2) `rootfstype=jffs2 ro
   zd1.slot=a|b zd1.good=a|b` to the command line and leaves the initramfs out; the kernel mounts the slot and runs
   its `/sbin/init` (`zd1-init`, `module/os/README.md`). The rescue: the initramfs, `zd1.rescue zd1.good=...`. Either way
   the command line starts with the boot image's own, which ends in `zd1.boot=ID`: its identity, by the stub's and
   the kernel's content (`module/os/assemble.py`; not the rescue's), reported as the State record's `boot`. No answer (the MCU's
   bootloader relaying an image, the stock MCU firmware): the rescue, since without the MCU the good slot is unknown
   and a slot may hold an old system or none; the rescue's `zd1-linkd` takes the bootloader's HCR 0x21 into BL2, or
   serves the MCU's bootloader the app it asks for (`docs/mcu/bootloader-hbl.md`).
2. **The MCU answers** a slot to try (once: the try is cleared as it is answered), else the good slot, else the other
   one, else the rescue, skipping the ones that failed since the last ready.
3. **Ready decides.** The module reporting ready (0x2100, `zd1-linkd`) makes the slot it booted the good one and
   clears the failures; at that ready the MCU also tells our OS the good slot (reg 0xC4 again: `zd1-linkd` keeps it in
   `/run/zd1/good`). A boot that doesn't get there fails: the stub asking again first (a root that doesn't mount
   panics, `panic=10` restarts the module), or no ready within 90 s (180 s for the rescue), when the MCU power-cycles
   the module. So a bad slot costs one boot and the good one starts again; a speaker with both slots empty goes A,
   B, then the rescue. A rescue that doesn't get ready is powered off.

`zd1 slot` shows the MCU's state; `zd1 slot try a|b|rescue`, `zd1 slot good a|b`, `zd1 slot clear`, with `--restart`
to start it now (`BootState` / `Boot` in `docs/protocol/host.md`).

**The rescue** is the system itself, from RAM: everything runs as from a slot (`zd1.rescue` on the command line, no
`zd1.slot`). With the slot an update would take empty (a first install: the slots are new), init's `self-install` copies
the running root into it (`zd1-slot install-self`: file by file, staying on the root's own file system, mount points
empty, the two device nodes a slot has) and stamps it with the version, so the next start is that slot's [HW
2026-11-19, `os.bcd` 6512879 with both slots erased: B and A failed, the rescue ready 60 s after the power-up, "slot b:
6512879 installed (10161 KB of files, 8660 KB of the partition used)" 29 s into its start, 20 s of copying; slot B
started next and got ready]. Whether a slot holds a system is read from its record (`/data/zd1/stamp.a|b`):
`zd1-slot erase` of a slot clears it, else the erased slot reads as the system it held.
A new boot image brings its system that way too, without a second upload: the host's `Boot INSTALL` (the page's
Update, `zd1 update os.bcd`, `zd1 install`, `zd1 slot install`) has the MCU start the rescue once with `zd1.install`
(its answer `boot::INSTALL`), and the rescue copies itself into the slot that isn't the good one (the good one if that
holds none; none if a slot holds its version already), then asks the MCU to start that slot once (`zd1-slot
install-self SLOT ID --start`: zd1-linkd's `try`, reg 0xD8); ready makes it the good slot, else the MCU goes back.
It is as new as the app partition's last install (BL2, the relay); a speaker whose slots both fail still runs and
plays, and takes an update as from a slot (`zd1 update` writes the good slot, `updater.md`).

**`zd1-slot`** (Rust, `module/os/apps/zd1-slot`): `status` (what runs, the good
slot, what each slot holds), `install SLOT ID` (a gzip'd newc cpio of a root on stdin: the partition erased as
`flash_eraseall -j` does, with JFFS2 cleanmarkers, mounted, unpacked, the stamp `/.zd1-system` written last; never the
running slot, nor the good one while another runs on trial), `install-self SLOT ID` (the system that runs from RAM), `mtd NAME` (its device nodes). A slot holds a system
when it has `/sbin/init` and its stamp: an interrupted install leaves none. The install also records the stamp in
`/data/zd1/stamp.SLOT` (empty from the erase until it ends): reading another slot's stamp needs no mount. A mount
scans the slot's JFFS2 in the kernel for ~12 s, which this kernel doesn't preempt: `zd1-linkd` stopped draining the
SPI slave meanwhile and an MCU write right after boot lost bytes [HW 2026-11-02]. A slot without a record is mounted
read-only once and recorded. One lock (`/run/zd1-slot.lock`) keeps an install from erasing a slot a read has
mounted.

**Data**: init mounts `/data`; one that isn't ours yet (no `/data/.zd1-data`: new, or the stock firmware's
leftovers) is formatted once. It is never reformatted for a new image, newer or older: every version keeps
`/data/.zd1-data` and reads what another version wrote (the settings: `settings.md`, Another version's settings).

The emulator's dev boot (`zemu run`) is the system as one initramfs, `/init` its `/sbin/init`, without the slots flag
in the payload: the stub boots it as it is (`zd1-ctl status`: no slot).

**The Lightning model** (`os-zd1l.bcd`, the module part of its `.fw`, the one way in being its BL2's web updater:
`docs/mcu/stock-firmware.md`) gets the same image with its HWID and CnE: its first start after BL2 installed it goes A,
B, the rescue, the self-install, and from then on it updates a slot at a time.

## How the system gets there
`zd1 update` (`docs/module/updater.md`): the image (`system.cpio.gz`, a slot's root) over USB through the MCU to
`zd1-update`, which writes the slot that doesn't run (from the rescue: the good one) and stamps it; then the slot is
started once. The `.bcd` that BL2 or the relay install carries the stub, the kernel and the system as the rescue: a
speaker whose slots are empty (its first install) boots slot A, slot B, then the rescue (each empty slot is one
failed boot: a panic and `panic=10`'s restart, ~15-20 s) [HW 2026-10-26, 2026-11-19], which fills the good slot (slot A
on a new speaker) [HW 2026-11-19]. Through the ZD1's USB relay (~23 KB/s) the 6.6 MB `os.bcd` takes ~5 min (the barebones rescue's 1.9 MB
took 85 s).

## On the unit [HW 2026-10-26 on]
The stub's question and the MCU's answer, empty slots failing (`panic=10` restarts the module), the rescue, a slot
installed and started on trial, confirmed by its ready; every update since into the slot that doesn't run (5 MB:
82 s through the MCU). Not yet seen: a trial that fails and falls back.

## Open
- Bad blocks in the slots and `data`: JFFS2 skips the ones the kernel's scan finds; none have been seen on this chip
  outside the factory list (`boot-chain.md`).
