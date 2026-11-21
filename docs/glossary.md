# Glossary

| Term | Meaning |
|---|---|
| ZD1 | B&W's codename for the Zeppelin Air (30-pin dock version); also this project's name (zd1) and its MCU firmware's |
| BCO, BridgeCo | the module's maker (later SMSC); "BCO" in the MCU firmware = the network module |
| CR870 | the plug-in network module; CR870-2C is the Zeppelin's variant |
| carrier board | B&W's board the CR870 plugs into; a 14-pin link to the MCU board, a cable to the back board |
| DM870, Sulu | the module's SoC (ARM926EJ-S); "Sulu" is BridgeCo's family name, also in Nokia's Linux (`mach-sulu`) |
| JukeBlox (JB) | BridgeCo's networked-audio platform/SDK; build 9112 = 0x2398 on this unit |
| KnOS | BridgeCo's RTOS in the stock module firmware (3.2) |
| WILD | the DM870's on-chip Wi-Fi core (Eagle MAC, its own ARM7) |
| Warp, Sol | the DM870's two 160 MHz MIPS cores (Warp0 secure/DRM, Warp1 audio) and the DSP datapath Warp1 drives (`docs/module/warp.md`) |
| SuluDMA | the DM870's DMA engine (packet and block moves) |
| AV port 2 | the DM870 audio output (I2S to the main board's DSP) |
| pad switch matrix | DM870 block at 0xE0013000 that connects pads to peripherals; the Nokia kernel never sets it |
| BL1 / BL2 | module first-stage bootloader (NAND block 0, encrypted) / second-stage updater (NAND 0xA0000) |
| stage 1, the module's preloader | the 8 KB SRAM loader inside every app/BL2 image |
| the ZD1 preloader (zd1pre) | our MCU code at 0x80004000, between B&W's DFU bootloader and the app (0x80008000): starts the app, or takes one from the module over SPI XModem (`docs/mcu/bootloader-hbl.md`) |
| Net HBL | the ZD1L's MCU bootloader (B&W 2.07): no USB; takes the app from the module's BL2 (or our OS playing BL2) by XModem |
| ZD1, ZD1L | the Zeppelin Air (30-pin dock, 2011) and the Zeppelin Air with Lightning (2013) |
| boot stub, linuxboot | our bare-metal stage in front of the Linux kernel (`module/boot`); after our MCU's knock (reg 0xC4 = 0xFFFF) it asks which slot to start |
| `zd1.boot=`, `zd1.image=` | the boot image's identities on its command line: the stub and kernel (whether a release rewrites the boot image), and the whole image with the rescue (what the MCU records) |
| `.bcd`, bCoD | module firmware container (app + factory CnE + resources) |
| MetaInfo | the 0x80-byte partition header in NAND |
| CnE | the module's configuration database (factory part at 0x40000, user part at 0x60000) |
| PersParam | the module's persistent parameters block (MAC, HWID, CnE pointers) at NAND 0x20000 |
| BLDb | the PersParam key that makes BL1 start BL2 at every boot |
| HWID | hardware id checked by BL2 (0xBD for this product) |
| HCR / DCR / DRC | host control (MCU → module commands, reg 0x28), device control (module → MCU, 0x2A), remote-display commands (0x48) |
| NAP | the module's audio player state (Searching, Connecting, Buffering, Playing) |
| RAOP | AirPlay 1's audio protocol (RTSP on TCP 1024 on the stock module); DACP = its remote-control side |
| MFi | Apple's accessory authentication (the auth coprocessor chips) |
| iAP | Apple's iPod accessory protocol (dock) |
| DFU, ISP | USB firmware update mode of the MCU bootloader; ISP_FORCE = the fuse bit that forces it |
| ISP word | the user-page word (0x929E0870) the bootloader validates |
| `module esc`, the console | `zd1 module esc` (`Module ESC`): BL1's serial menu into BL2 over the module console, the MCU driving it. `zd1 console`: a terminal on the module's console over the host link |
| host link | our MCU firmware's typed requests and log over USB (a vendor interface on EP0), and the same messages over the network through our OS (`zd1-netd`): `zd1`, `docs/protocol/host.md`, `docs/protocol/net.md` |
| the network link, pairing | `zd1-netd` on port 80 (WebSocket + Noise): paired clients reach the speaker over the LAN; a client pairs once by ON/OFF held 5 s |
| the hold, `Allow`, the allowance | ON/OFF held 5 s on the speaker: confirms a pairing and a network update; the update's (or `zd1 allow`'s) lets the MCU take one app from the module, until 60 s after the module's next ready |
| the MCU follows its OS | each system carries the same build's MCU app; on a confirmed slot whose app differs, our OS gives it to the MCU's bootloader (under an allowance) |
| system slot, rescue, data partition | our module OS on the NAND: two slots (`system-a`, `system-b`), each a whole root the kernel mounts read-only; the rescue, the release's whole system as the app partition's initramfs: it runs when no slot gets ready or no MCU of ours answers, and installs itself into a slot (self-install, INSTALL) on a boot image our MCU hasn't recorded; `/data` (settings, the AirPlay identity), all JFFS2: `docs/module/storage.md` |
| zd1, cr870 | the two emulated machines (Renode, `emu/README.md`) |
| zemu | the emulator driver (`emu/zemu`): `zemu run IMAGE` boots and drives an image |
| zd1-link, zd1-linkd | our module-side link code (`module/link`, the protocol library); zd1-linkd = the link under Linux (`module/os/apps/zd1-linkd`) |
| native dialect | the registers our MCU firmware and our OS use between them (0xC0-0xC6, 0xD2-0xD9) instead of the stock app's DRC codes, volume and play-state XML: `docs/protocol/sequences.md` |
| Warp0 runtime | our firmware on Warp0 (crypto, AAC, Vorbis, FLAC) and its ARM client: `module/warp0/` |
| unit | one physical speaker; per-unit data lives in `units/` |
