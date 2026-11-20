# The MCU's own firmware from the module: the ZD1L's bootloader and our ZD1 preloader

Both models take a new MCU app the same way: the MCU's bootloader powers the network module, asks it for BL2 and
receives the app over the SPI link as XModem. On the Zeppelin Air with Lightning (ZD1L) that bootloader is B&W's
("Net HBL"); on the Zeppelin Air (ZD1) it is our preloader, which speaks the same. The module side is our OS:
`zd1-linkd` answers as BL2 would, from an app it has staged (a `.dmb`). One app build, at 0x80008000, runs on both.
Tags: `docs/README.md`.

## The ZD1L's bootloader ("Net HBL" 2.07) [RE]
Read from a ZD1L running our firmware (`zd1 peek`, jurajs5's release `mcu-zd1l-dump`, 2026-11-18;
`blobs/zd1l/mcu/bootloader.bin`, the first 32 KB). Disassembly and decompile: `re/gen/mcu-bl-zd1l/` (gitignored; made
with the AVR32 toolchain's objdump and Ghidra over an SRAM image, scripts there).

| | |
|---|---|
| 0x80000000-0x800048FF | the bootloader: the reset decision, then a copy of 0x800001AC-0x800048FB to SRAM 0x2000 that runs (entry 0x2F3C) |
| 0x80004900-0x80007FFF | zeros; lock regions 0-1 (the flash controller's status read 0x00036001: 256 KB, LOCK0 and LOCK1, security bit clear) |
| 0x80008000- | the app, started directly (no branch at 0x80004000, no check of the app) |

- **No USB.** It never touches the USB controller (the ZD1's DFU bootloader does, 72 times): the only way an app gets
  in is from the module, and JTAG.
- **When it runs**: the ZD1's reset decision, word for word (`bootloader-dfu.md`): ISP_FORCE, ON/OFF held at power-up
  with the factory ISP word, or "ISPK" in SRAM after a warm reset.
- **It clears ISP_FORCE at once**, sets it again only while it writes the app (a power cut in the middle comes back
  here), and clears it on its way out. So one power cut mid-write is retried; a second boots what is in flash.
- **What it does**, a state machine polled with the link (its log on USART0, PA01, 115200: "*** ZD1: Net HBL 2.07
  (Nov 19 2012 - 11:33:12) ***", "BSL Ready", "DownloadFirmwares()", "ReceiveHostFirmware() - Done", "Restart", "Reset"):
  1. Power the module (PA04, PA17), wait for ready and its version reads (0x25-0x27, then 0x32: its "info ready").
  2. Configuration channel (0x45): `V HWID 0xBE000000` (the persistent parameters; no answer awaited), `R
     Application/NetIF/mode`; anything but "autosense" is written so with `W`.
  3. HCR 0x2120 (BL2), unless the module already says BL2 (status 0x2300).
  4. Wait for BL2 (0x2300) and its "info ready", then for DCR 0x2A = 0x2423 ("DownloadFirmwares": BL2 takes the
     web upload meanwhile; HCR status 0x22xx is progress, anything but 0x2020/0x2120/0x214E a failure).
  5. XModem receive (`C`, then ACK / NAK / CAN on 0x2D; 128-byte SOH blocks with CRC-16 read from 0x2C; EOT): the
     `.dmb`'s header first, kept in RAM; its app into 512-byte pages from 0x80008000 (only the pages it covers: the
     rest of the flash keeps what was there); the header's length and CRC-16/XMODEM checked after the last page.
     Nothing else in the header is looked at. A good transfer also needs an HCR status OK after it.
  6. Status 0x2500 (BL2 finished), 10 s, the module off (PA17, 3 s, PA04), a watchdog reset into the app.
  ON/OFF held 3 s in most waiting states restarts into the app. With no module answering it waits for that press.
- The reference trace: the real BL2 (`blobs/zd1l/module/bl2.bin`) and this bootloader in the emulator, `bl2web` with
  B&W's 2.04.35 `.fw` [EMU 2026-11-19; `re/gen/mcu-bl-zd1l/hbl_bl2_trace_spi.txt`]. The emulated transfer broke at
  block 0x1E (BL2 skipped a block); on a real ZD1L it works [HW, the owners' installs].

## Our side: zd1-linkd plays BL2 [EMU]
`module/os/apps/zd1-linkd/src/mcufw.rs`. With a good `.dmb` in `/data/zd1/mcu/pending.dmb`, an HCR 0x2120 from a host
that never greeted us (not our MCU firmware) and that used the configuration channel as the bootloader does (`HWID`,
`Application/NetIF/mode`, answered "autosense") is answered as BL2, without rebooting: status 0x2000, 1.5 s, 0x2300;
3 s later 0x29 = 0x2020 and 0x2A = 0x2423; then the blocks on 0x2C as notifications, 0x29 = 0x2020 again with the
first; after the EOT's ACK 0x29 = 0x2020 and, a second later, status 0x2500. The HBL wants an OK it has taken after
its receive began (its download step clears the one before), and takes none once the status says finished (a status
change is always read first): with the OK and the status together it finished the transfer and never said Done
[EMU 2026-11-19]. Without a staged image the HCR reboots into BL2 as before (the bootloader's relay, `zd1 module bl2`).
A bootloader quiet for 7 s mid-transfer gets the block again; 30 s ends it.

- **Split escapes** [EMU 2026-11-19]: a transfer stopped at block 580: an escape pair (`27 27`) went out as `27`,
  idle bytes, `27`. The pair's second byte, written after the MCU had clocked out everything ahead of the first, came
  behind idle bytes; the HBL waited 5 s for the bytes it then missed and gave up the read. A block offered again 7 s
  later is taken and the transfer runs to its end, but the HBL's update doesn't: it never says Done (a failed read
  seems to cost it its main state machine). So pairs must not split: `module/link` queues 48 bytes (idle ones if
  need be) ahead of every pair, 128 us of the MCU's clock, more than an interrupt between the two writes. The
  emulator runs the two boards in 200 us slices, in which the MCU clocks more than the whole FIFO, so these runs want `--quantum-us 100` (at 50 the stub's handshake with our MCU fails in the emulator; at 200 the slot answer works);
  the unit has no slices, and BL2, bare metal, never splits one [HW: the owners' installs].
- **The module must stay silent until the HBL's first look finds `zd1-linkd`**: our boot stub, answering the HBL's
  status reads with 0x2000 while it waited for a slot answer that never came and then going quiet for the kernel's
  boot, sent the HBL into its dead end (waiting for a key) [EMU 2026-11-19]. The stub now keeps the link muted until
  our MCU knocks (`docs/module/storage.md`, the boot); the HBL never knocks, so under it the stub starts the rescue
  without a word (on a ZD1L the full system) and the HBL's first contact is `zd1-linkd`.
- **The staged image must be on the NAND before the MCU goes**: an image fetched seconds before lost its tail in the
  power cut on the MCU's way to its bootloader [EMU]; `mcu-update` syncs first.
- **End to end** [EMU 2026-11-19, `zemu run` of the ZD1L test NAND (its BL2, HWID 0xBE) with `os-zd1l.bcd`, the HBL and
  `.#mcu-emu`'s app in the MCU flash, board variant 2, `--quantum-us 100`]: the first start's self-install, then
  `zd1-ctl mcu-update` with B&W's 2.04.35 `.dmb` staged: "[mcufw] into B&W's bootloader", the stub silent ("no knock"),
  HWID, "BSL Ready", the download step, 1539 blocks without a stall in 23 s of the module's time,
  "ReceiveHostFirmware() - Done", "Restart", "Reset", and B&W's app ran (to RUN; it then asserts on our OS's link, as
  the stock app with our OS always does).
- How our OS starts it: `zd1-ctl mcu-update` (or anything that writes `mcu-update` to /run/zd1link) checks the staged
  `.dmb` and notifies 0xD7 (`MCU_FW`, `docs/protocol/registers.md`); our MCU firmware takes it once, until 60 s after the module's next ready (10 minutes at most), after
  a person allowed it (the host link's `Allow`: ON/OFF held 5 s, asked at a network update's start; else it refuses,
  and by hand `zd1 allow` over USB first), and goes where it is taken
  (`mcu/src/mcufw.c`): our preloader on the ZD1 (GPLP0 bit 25, the good slot in GPLP1 for our boot stub, the module
  off, a watchdog reset), the HBL on the ZD1L (ISP_FORCE: `selfdfu_enter`). The image stays staged. The system
  asks by itself when the MCU runs another version than the app it carries (`/usr/lib/zd1/mcu/zd1.dmb`, the same
  build's), at the MCU's hello on a confirmed slot, until the bootloader has taken that version twice: the MCU follows
  its OS (`docs/protocol/net.md`; `zd1-linkd`, `mcufw::follow`).
- **Why a person confirms it**: neither bootloader checks more than a CRC, and the MCU's firmware is what keeps the amp
  safe (its heat trips, the volume cap). A module someone took over over the network (it parses what the network
  sends, as root) could otherwise give the MCU a firmware without them. Both bootloaders start only when our app
  sends the MCU there (or by hand: ON/OFF at power-up), so the app's wait covers the ZD1L's B&W bootloader too.

## Our ZD1 preloader [HW 2026-11-19]
`mcu/preloader/preloader.c`, 0x80004000-0x80007DFF (7 KB), started by B&W's DFU bootloader where the app used to start;
the app moves to 0x80008000 on the ZD1 too (`mcu/Makefile`; one build for both models). Its record of the app is the
page at 0x80007E00: the app's `.dmb` header, written after a transfer checks, erased before the first page of one.
- No update asked (GPLP0) and a record: straight into the app, nothing touched (RC oscillator, the pins as reset left
  them) [HW 2026-11-19: 6512879's `zd1.bin` over B&W's DFU (`mcu/flash.sh`, from our app at 0x80004000): the app
  greeting from 0x80008000, its boot log as before; the module and our OS as before].
- Else: the PLL, USART0 for its log (`[pre] ...`), the module powered, ready awaited (our boot stub's slot question
  answered with what the app left in GPLP1), `R Application/NetIF/mode`, HCR 0x2120, BL2 and the DCR awaited, the
  record erased, the XModem receive as the HBL's, the CRC checked, the record written, then a watchdog reset into the
  app. A failure before the record is erased goes back to the old app after three tries; after it, the preloader
  keeps asking (every start comes back here while there is no record). B&W's DFU (ON/OFF at power-up) stays the way
  back from anything, the preloader included.
- It reads the module's notifications in their order, the pending register first, as the HBL does: our OS queues
  BL2's HCR status OK ahead of the first block, and a direct read of 0x2C past it answers the register's idle value
  (`00 00`), so a preloader reading 0x2C straight never got a block and gave up every transfer [EMU 2026-11-19: our
  system's carried `zd1.dmb`, 579 blocks, in flash and checked once it read in order].
- **It starts the board and powers the module as the app does** (`mcu/src/zd1_board.c`, linked into both): the
  bootloader's pin state on the reset clock, then the clocks in the bootloader's order; the module's supply, 1200 ms,
  the reset released and SPI on, 1200 ms before the first read; the SPI pins driven only while the module is powered.
  Its first build on the unit started ASF's clocks first, drove the SPI pins with the module off and released the
  reset 100 ms after the supply: the module never came up, and the speaker stayed dark (no USB, no LED, no network)
  until DFU, its flash untouched [HW 2026-11-19]. Which of the three did it is not known; the emulator showed none.
- **Its LED**: stock's colour for a firmware update, white, blinking slowly (1 s) while it waits for the module and
  quickly (200 ms) while the app comes in, drawn from `wait_ms` (every wait passes there); the PWM is the app's
  (`mcu/src/led_pwm.c`) [HW 2026-11-20, 4b7baee: the slow blink while the module started, the quick one through the
  transfer, the app back 46 s after it left].
- **Its report**: it leaves in GPLP0 (kept over the reset) the tries and how the last three ended, and after a failed
  one in GPLP1 how the module looked (the last status's high byte, whether it knocked, whether it answered the stub);
  the app logs it at its start and clears it: "[pre] the preloader's last run: 2 tries: the module never got ready;
  took the app; the last try that failed: the module's status 0x20xx, knocked, no slot question answered" [HW
  2026-11-20: the report of real runs, and of a run written into GPLP by hand]. A first try waiting out its 180 s and
  the second taking the app happened twice on the unit (2026-11-20, before the module's view was kept).
- **On the unit** [HW 2026-11-19, the preloader of 249a535 in flash, its app 249a535, the system 6512879 on its
  confirmed slot B]: at the module's ready the system asked for its own MCU app (the MCU follows its OS), the MCU went
  off USB, and 45 s later the system's app 6512879 greeted with the record at 0x80007E00 (0x120A2 bytes), the module
  ready again 22 s after.
- In the emulator [EMU 2026-11-19, `zemu run` of the test unit with `os-zd1l.bcd` and `.#mcu-emu`'s `zd1emu.bin`
  behind B&W's DFU bootloader]: `zd1-ctl mcu-update` with B&W's 2.04.05 app (cut at 0x80008000, as a `.dmb`) staged:
  "[mcufw] into the preloader", "[pre] BL2 has an app: taking it", "app in flash and checked, 0x000300f4 bytes", the
  restart, and the stock app ran ("Device: ZD1 User mode: NORMAL"); 1539 blocks in 22 s of the module's time. With
  nothing staged (the real 2010 BL2 answering instead): "BL2 has no app for us" three times, "giving up: back to the
  app we have", our app as before.
- `nix build .#mcu` makes `zd1.bin`/`zd1.hex` (the region from 0x80004000: preloader, record, app: what `zd1 flash`,
  the page and zemu take), `zd1.dmb` (what the module serves) and `zd1app.bin` (for `bwfw.py --pack`; `mcu/stamp.py`).
