# cfg80211 and mac80211 from source

The module's Wi-Fi stack above the radio's driver: `cfg80211.ko` and `mac80211.ko` for Nokia's 2.6.24.7 kernel,
built here (`nix build .#wifi`, `nix/module/wifi.nix`) in place of Nokia's binaries. `sulu_wlan.ko`, the radio's
driver, stays Nokia's: this is the code it was made for, so it fixes the interface between them (`ieee80211_ops`,
`ieee80211_hw`, `ieee80211_conf`, the RX and TX status) and everything above it is ours to change.

## The source
Linux's wireless code as of mainline `1ebebea8e8` (2008-04-23): `net/mac80211`, `net/wireless` and the headers they
bring (`include/`). Nokia built theirs from a compat-wireless snapshot of wireless-testing of about 2008-04-22, which
no longer exists (`re/netmodule/NOKIA_HD1.md`, Wi-Fi, has how it was dated); this commit passes every check made
against their binary. Then our changes, in this repository's history:

- Nokia's field in `struct ieee80211_conf`, `user_power_level` (the TX power fixed by SIOCSIWTXPOW, 0 = the
  channel's maximum), and its two uses (`ieee80211_ioctl_siwtxpower`, `ieee80211_hw_config`): their `sulu_wlan` is
  built on that layout.
- `config.h`: this build's options. No mesh: the draft mesh code takes an element 51 (today the AP Channel Report)
  for its mesh configuration and drops the beacon of an AP that sends one (issue #11).
- `include/net/compat.h` and an `#ifdef` in `wext.c`: what 2.6.24.7's wireless headers lack.

Nokia's own two changes are not here: the WPS element in scan results (`wps_ie=`) and freeing BSS entries not heard
for a while. Nothing here uses the first.

## The build
`kernel.py` prepares vanilla 2.6.24.7 (linux-stable's tag, `os.toml [wifi]`) for out-of-tree modules: Nokia's
configuration from the kernel image (`IKCFG_ST`), a stand-in machine (`ARCH_SULU`: an ARM926T, SDRAM at 0x60000000,
Versatile's other headers), the Makefile rules make >= 3.82 rejects, and what clang needs. After `oldconfig` the
configuration differs from Nokia's only in their Sulu drivers, filesystems and drivers these modules don't touch.
Then kbuild builds `Kbuild` here as compat-wireless did (both modules in one pass, so `mac80211` depends on
`cfg80211`), with clang and lld, on Linux (the tree has names that differ only in case).

What a module needs to load into Nokia's kernel, and what this one was checked for: `vermagic=2.6.24.7 mod_unload
ARMv5` (no symbol CRCs: no MODVERSIONS), `struct module` 0x160 bytes, relocations the kernel's `apply_relocate`
takes (R_ARM_ABS32, PC24, CALL, JUMP24: no PIC, no unwind tables; clang's `.ARM.exidx` is removed), imports only
what the kernel and Nokia's modules export (no `bcmp`). The structures shared with `sulu_wlan` were checked against
Nokia's build: the offsets every function touching them uses (built with GCC like theirs), and `sulu_wlan`'s own
(`ieee80211_ops` filled at the same indices, `hw->priv` at 0x48, `conf->channel` at 0x1c, a 0x3c-byte `conf`, a
0x40-byte TX status).

## On the unit
[HW 2026-11-21, system 4557b09] Loaded at run time in place of Nokia's (`zd1-ctl stop wifi`, `rmmod sulu_wlan
mac80211 cfg80211`, `insmod` these, `sulu_wlan diversity=2 macaddr=$(zd1-persparam mac --wlan)`, `zd1-ctl start
wifi`): the firmware starts, the scan lists what Nokia's lists, WPA2-PSK CCMP joins, DHCP, the router answers every
ping, and the Mac's pings fare as with Nokia's modules. A reload makes the module leave the AP (reason 3); the
authentication requests then go unanswered for minutes, with Nokia's modules as with these (`docs/status.md`).
