# Binaries kept in the repo

The vendor binaries the tools, the emulator, the builds and a recovery need: B&W / BridgeCo firmware from 2010–2015
and the pieces of Nokia's Linux our module OS runs on, kept here so that a checkout is complete. None of them holds
per-unit data (MAC, settings, Wi-Fi): that lives in `units/` (gitignored) and is rebuilt from a MAC by
`tools/zeppelin/recover/mkunit.py`. By model: `zd1/` the Zeppelin Air, `zd1l/` the Zeppelin Air with Lightning; what
both share: `dm870/` (B&W's module images' common part) and `nokia-hd1/` (the Linux our module OS runs on).
`tools/zeppelin/paths.py` names them for the tools.

## `zd1/`: the Zeppelin Air

| File | Bytes | sha256 | What | Source |
|---|---|---|---|---|
| `mcu/bootloader.bin` | 16384 | `b4d1d810…13dddf1` | Atmel/B&W DFU bootloader, MCU flash 0x80000000-0x80003FFF (BOOTPROT) | DFU read of our unit, 2026-10-16 (LOCK unset, direct read) |
| `mcu/zd1-2.04.05.hex` | 644903 | `ca307164…32d164` | stock MCU app 2.04.05, Intel HEX from 0x80004000 | B&W `zd1v20405.zip` = the Recovery Utility's copy; a DFU readback of a stock unit is this plus 0xFF |
| `mcu/dmhFixZD1d.hex` | 383034 | `547e7f8c…494085b` | "DMH fix" MCU image the Recovery Utility flashes first (`dmhRecoveryApp` v2.00). In the emulator it erases the user page, sends the module HCR 0x2421 (factory reset of the user settings) and holds PB01 high [EMU]; never run on the unit | B&W Zeppelin Air Recovery Utility 2.2.2 (Mac) |
| `module/bl1.enc` | 65536 | `b2d406d2…dc7c860` | module BL1 as stored in NAND block 0 (encrypted, ROM-decrypted; used extent 0x869C) | `fburn rd` of our unit; identical to the archive.org dump's block 0 |
| `module/bl1.plain` | 34464 | `fba3b527…f7eb7654b` | BL1 decrypted, as it runs at SRAM 0x400000 | SRAM capture at stage-2 entry (`re/netmodule/bl1/README.md`), 2026-10-19 |
| `module/bl1.run` | 35336 | `58f30c4d…f2e9b92c9` | BL1, re-runnable: the stage-2-entry SRAM capture (`re/netmodule/bl1/sram_at_stage2_entry.bin`) cut at BL1's end 0x8A08, the scatter-load call at 0x400010 replaced by `b 0x400688` (the C entry; its data is already unpacked, in the post-boot state). What `zemu run --nand --real-bl1` executes | made from that capture (`re/netmodule/bl1/README.md`), 2026-10-23 |
| `module/bl2.bin` | 798720 | `fefd9a49…30181c` | module BL2 partition from NAND 0xA0000 (MetaInfo header + BL2, CRC OK) | `fburn rd` of our unit |
| `module/app_DMP.bcd` | 6847488 | `568bc824…f8b11b8dd` | stock module image (app + factory CnE + resources), JukeBlox 3.8.0.9112 | B&W `zd1v20405.zip` / Recovery Utility |
| `module/cne.bin` | 69096 | `ff4bed39…6e945592` | its factory CnE (`.bcd` section 1, `TCPIPTunnel = 'REMOTE_REGISTERS'`), the CnE of `os.bcd` | `app_DMP.bcd`, section 1 |

## `zd1l/`: the Zeppelin Air with Lightning

| File | Bytes | sha256 | What | Source |
|---|---|---|---|---|
| `zd1lv20435.fw` | 7047573 | `757a48bb…2006d6c9` | B&W's last release for it, 2.04.35 (March 2015), both chips: the stock module image and the MCU app `ZD1L_v2-04-35.dmb`. **The way back**: key-hold → BL2 → `python3 tools/zeppelin bl2web IP blobs/zd1l/zd1lv20435.fw --yes` (`docs/mcu/stock-firmware.md`) | B&W's firmware server, recovered 2026-11-04 (`downloads/bw-zeppelin-air-software/zeppelin-air-lightning/SOURCES.md`; also on archive.org, `downloads/manifest.json`); md5 `d71a10be…` as in B&W's manifest |
| `mcu/bootloader.bin` | 32768 | `0dc244ed…e0ca8120` | the MCU's bootloader region 0x80000000-0x80007FFF (lock regions 0-1): B&W's "ZD1: Net HBL 2.07" (Nov 19 2012, 0x0-0x48FF, zeros after). No USB: it takes the MCU app from the module over SPI (XModem from BL2, after DCR 0x2423) into 0x80008000 and starts it there directly; the same reset decision as the ZD1's (key-hold, ISP_FORCE) | read-only `zd1 peek` of a ZD1L running our firmware 0.4.2 (jurajs5, GitHub release `mcu-zd1l-dump` of jurajs5/zd1, 2026-11-18; whole-flash sha256 `62e11e4a…6db6`), its first 32 KB |
| `module/bl2.bin` | 1703936 | `2e6eb04f…57dfe` | its BL2 partition from NAND 0xA0000 (build 2012-11-06, BridgeCo MR5; `docs/module/boot-chain.md`, "The ZD1L BL2") | `fburn rd 0 0xA0000 0x1A0000` of a ZD1L owner's unit over the stock-patch telnet hook, contributed via GitHub issue #1, 2026-11-16 |
| `module/cne.bin` | 69084 | `aeb57ab5…09bbe952` | its factory CnE (`.bcd` section 1; differs from the ZD1's in `TCPIPTunnel = 'OFF'`): its BL2 writes it to the factory CnE at NAND 0x40000, so `os-zd1l.bcd` carries the ZD1L's own | `zd1lv20435.fw`'s `.bcd`, section 1 |


## `dm870/`: both models' module images

| File | Bytes | sha256 | What | Source |
|---|---|---|---|---|
| `preloader.bin` | 8016 | `77934b1d…4f4b76b1` | B&W's stage 1 (the SDRAM/clock/NAND preloader, run from SRAM at 0x40DA00): an app section's bytes 0x40-0x1F8F, behind the boot header `mkbcd.py` writes; the same in the ZD1, ZD1L and A7 images | `zd1/module/app_DMP.bcd` section 0 (= `zd1l/zd1lv20435.fw`'s, `a7v20477.fw`'s) |

## `nokia-hd1/`: Nokia's Linux for the module's chip

Nokia's Home Music / HD-1 runs Linux (BCoLinux 2.2.1, kernel 2.6.24.7 `mach-sulu`) on the CR870's SoC family; its
kernel and drivers are what our module OS runs on (`re/netmodule/NOKIA_HD1.md`, `module/os/os.toml`). Taken from a
third party's full NAND dump, archive.org item `nokia-hd-1-dump`, file `mtd0-11ro.zip` (sha256 `2f0582e3…f8c4d00`,
sha1 `ba009444…31994aad7`); unmodified, and checked against a fresh extraction (2026-10-25):

| Path | What | From |
|---|---|---|
| `kernel.bin` (2986200 B, sha256 `66763b00…e3d2eff`) | Kernel_A, the ARM Image | `mtd6ro` (sha256 `897b49c1…4649d83`): gzip stream at 0x3380, inflated |
| `rootfs/` | the part of Nokia's root filesystem we use: the 9 kernel modules in `os.toml [vendor]` (Wi-Fi, AV, sound, Warp), the Wi-Fi firmware, `.rootfs-version` (2009-10-01). | `mtd7ro` (sha256 `0b47be3e…0bc1f91`), JFFS2, extracted with jefferson |
| `wslsepp_warp.bin` (27268 B, sha256 `f44a9e34…0a6d00b6c`) | Nokia's WarpSol: the Warp1 program their app loads (linked at 0x61FC0000; `docs/module/warp.md`), for `tools/zeppelin/warp` | the symbol `wslsepp_warp` in `rootfs/home/root/JukeBlox_Release_ELX` (sha256 `33ea9045…1b904af02`; the app itself is not kept), cut by `tools/zeppelin/warp/warpblob.py` |

No per-unit data: the dump's settings partitions are not here.

Everything else that was downloaded (installers, other devices' firmware, documents) is on archive.org, listed in
`downloads/manifest.json` (`python3 tools/zeppelin downloads fetch`).
