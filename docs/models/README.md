# The models

Three B&W speakers share the main board's MCU (AT32UC3A1256), the network module (the CR870, ZZ34088) and the
stock firmware's code base, and our firmware runs on all three from the same builds: one MCU app, one module system.
What differs is told apart at run time, on each board from what the factory left there.

| | Zeppelin Air (ZD1) | Zeppelin Air with Lightning (ZD1L) | A7 |
|---|---|---|---|
| Dock | 30-pin (USB, composite video) | Lightning (USB) | none |
| Module HWID (PersParam, the `.bcd` header) | 0xBD | 0xBE | 0xA7 |
| MCU bootloader | B&W's DFU bootloader (USB), 0x80000000-0x80003FFF (`mcu/bootloader-dfu.md`); our preloader behind it (`mcu/bootloader-hbl.md`) | B&W's "Net HBL" 2.07, 0x80000000-0x80007FFF, no USB; writes HWID "0xBE000000" into the module's PersParam at every start (`mcu/bootloader-hbl.md`) [HW dump] | never read; its key-hold reaches BL2 as the ZD1L's does [HW, an owner, read-only]; taken to be a "Net HBL" writing "0xA7000000" (`a7.md`) [INF] |
| B&W's last MCU app | 2.04.05 (`.hex`) | `ZD1L_v2-04-35.dmb` in `zd1lv20435.fw` | `AZ1_v2-04-77.dmb` in `a7v20477.fw` |
| DSP setup (ADAU1445) | the 2.04.05 download | the ZD1's, byte for byte [EMU] | its own program's output stage and voicing; the same controls (`a7.md`, DSP) [RE, EMU] |
| USB name | "Zeppelin Air" | "Zeppelin Air" | "A7" |
| Our first install | over USB, from its DFU bootloader (`zd1 install`, the page) | `zd1l.fw` through its BL2's web updater, from key-hold (`mcu/stock-firmware.md`) | `a7.fw` the same way, once `bl2web`'s dry run finds its BL2's expert confirm (`a7.md`) |
| The way back | B&W's update file through DFU, or a backup | `blobs/zd1l/zd1lv20435.fw` through BL2 | `blobs/a7/a7v20477.fw` through BL2 |
| Ours on one | [HW] the owner's unit | [HW] owners' units (an earlier build; `status.md`) | never: emulator only [EMU] |

## How our firmware tells them apart

- **The MCU** (`mcu/src/model.c`), first thing at its start, before USB and the DSP: no B&W "Net HBL" in
  0x80000000-0x80007FFF (B&W's DFU bootloader, with or without our preloader behind it) is a ZD1. With an HBL, the
  HWID string it carries (exactly one `0xHH000000` between NULs; the ZD1L's at 0x800044CC) says which: 0xBE the ZD1L,
  0xA7 the A7. An HBL with no string or another HWID is **unknown**: the DSP stays in reset (no model's crossover is
  right for drivers we don't know) and the amp stays off (`amp_may_run` asks for a DSP that was set up). The bootloader
  is the factory's, in the flash's locked regions, and nothing of ours writes it (`AGENTS.md`, protected regions), so
  the answer can't drift with an install. The host link reports it (`InfoReply.model`, `hbl_hwid`): `zd1 info`, the
  page's Software panel.
- **The module** (`zd1-persparam`'s `unit_model`, `zd1_sys::Model`): the HWID record in its persistent parameters,
  the factory's (and BL2's default, 0xBD, for a store it had to rebuild). It names the speaker the way the model's
  stock software did: "Zeppelin Air XXXXXX" or "A7 XXXXXX" by default, the AirPlay model "Zeppelin Air" or "A7".

The ZD1 and the ZD1L get the same DSP setup and the same names; on them the model changes only the MCU firmware's
way in (`mcu/bootloader-hbl.md`) and the refusal of USB DFU on the ZD1L (no USB in its bootloader).

## The first install's file

`nix build .#zd1l-fw`, `.#a7-fw` (`nix/bw-fw.nix`): a B&W over-the-air `.fw` with both chips, which the model's BL2
web updater takes (`mcu/stock-firmware.md`, "Our firmware for a ZD1L"). The module part is `os-zd1l.bcd` or
`os-a7.bcd` (`module/os/assemble.py`: the whole system as the boot initramfs, the model's HWID, which BL2 holds
against the speaker's, and its factory CnE from `blobs/`); the MCU part our app, the same for every model, under the
name and version of B&W's last release for it. Releases carry both (`zd1l-VERSION.fw`, `a7-VERSION.fw`); the page's
release manifest has them by model (`web.md`, Releases), and its Install offers each behind a tick.
