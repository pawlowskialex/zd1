# Patching the stock MCU firmware

`patch.py` puts the telnet hook into the stock Zeppelin Air MCU app (2.04.05, `blobs/zd1/mcu/zd1-2.04.05.hex`): once the
module is ready, the MCU writes reg 0x75 = "23" and reg 0x76 = 5, the module's KnOS shell on telnet port 23,
re-asserted every ~30 s (`telnet_hook.S`). Python only:

```
python3 -I stock-patch/patch.py out.hex                                # ZD1 2.04.05 -> a .hex for DFU
python3 -I stock-patch/patch.py --fw blobs/zd1l/zd1lv20435.fw out.fw   # ZD1L 2.04.35 -> a repacked B&W .fw
python3 -I stock-patch/patch.py --fw a7v20477.fw out.fw                # A7 2.04.77 -> a repacked B&W .fw
```
Flashed and checked on the unit 2026-10-16 (2.04.05). It is not on the unit now: that runs our own firmware (`mcu/`),
which has the amp interlock built in and puts the shell on telnet with `zd1 reg write` (`docs/module/knos.md`).

## Three builds, one blob
`voTask` (0x8000e19c) and `WRITEREG` (0x8000b910) keep the same address in ZD1 2.04.05, ZD1L 2.04.35 and A7 2.04.77,
so the hook's code is identical across them; only four literals it uses are build-specific. `patch()` keys on the
crt0 bss-end value (`memend`, the word at 0x80008088) and relocates those four in the committed blob — the module-comm
`STATUS` word and attach-state byte the hook gates on, and the two carved-RAM addresses, which follow `memend`:

| `memend` | build | `STATUS` | `ATTACH` | carve | heap literal |
|---|---|---|---|---|---|
| 0x8250 | ZD1 2.04.05 / ZD1L 2.04.35 | 0x3ee0 | 0x4df4 | 0x8250 | by value (0x8002ae28, or 0x8002ae24 on ZD1L: the -4 data shift) |
| 0x7a08 | A7 2.04.77 | 0x3918 | 0x482c | 0x7a08 | by value (0x8002a3ec) |

The voTask call pointer (`== voTask`'s address, unique in flash) and sbrk's heap-base literal (`== memend`, unique in
0x8002A000-0x8002B000) are located by value, not address, so the −0x5c8 RAM shift and the moved pointer in A7
(0x8001e870 -> 0x8001de48) need no hardcoding. An unrecognised build (unknown `memend`, e.g. the older, structurally
different ZD1L 2.01.04) is refused rather than patched wrong. `--fw` patches the MCU `.dmb` inside a B&W over-the-air
`.fw` (`tools/zeppelin/fw/bwfw.py`) and repacks it, the module `.bcd` untouched; the `.dmb` links at 0x80008000, so
there is room for the hook at the cave (free in all three: each app ends well before it).

What the emulator showed for A7 [EMU 2026-11-14: `zemu run module.bcd --mcu FLASH --renode 'zd1:board Adc "2:0x100"'`,
the `.fw` split by `bwfw.py`]:
- The patched MCU boots on the ZD1 board model ("Device: AZ1") and reaches RUN identically to stock, with the 8-byte
  carve live (free heap 16812 B vs stock 16820 B); the A7 module image boots the full KnOS stack and gets networking.
- **The hook fires.** Watching the carved STATE byte (`zd1:board Watch "0x7a08:4"`) shows it advance 0 → 1 → 2 → 3 → 4
  over t=5.3-6.3 s — the whole write sequence (reg 0x75 = "23", reg 0x76 = 5), reaching the enabled state. For the
  state machine to leave state 0 it must have seen `STATUS(0x3918) == 0x2100` and `ATTACH(0x482c) == 1`, so the
  relocated A7 addresses are right and the gate opens. Telnet port 23 then accepts a TCP connection (`net.log`:
  `tcp open host -> :23`).
- Interactive shell output over the connection is empty in the emulator — but **identically for the known-good
  ZD1L**, so it is an emulator usernet / shell-routing limitation, not an A7 or patch issue.
- Both stock and patched A7 (and ZD1L) log a *pre-existing, non-fatal* assert at ~21 s (`dmhBcoMgr_ercGetCneValue`,
  `dmhBcoMgr_Cne.c:149`): the MCU fails to read one CnE value from the module because the emulator boots the image
  against the test unit's PersParam (NAND 0x20000) and an uninitialised persistent-CnE region (NAND 0x60000,
  `fixpage result=6`). The hook fires straight through it. A real A7 unit, with its own factory data, would not hit
  it.

So the A7 payload is built, verified static, and verified to fire end-to-end up to the listening telnet port; a live
interactive shell is unproven in the emulator (the ZD1L limitation above) and awaits an A7 unit (`docs/backlog.md`).
Delivery is the other open part: what on a ZD1L or an A7 takes a `.fw`'s MCU part is unconfirmed
(`docs/mcu/stock-firmware.md`), so these are the payload for that route, not proof the route exists.

How: the BcoWrapper tick calls `voTask` through a pointer literal; the patch points it at the hook in free flash at
0x80038100, which runs a small state machine and then calls the real `voTask`. One byte of state at RAM `memend`
(crt0's bss end and the heap base move +8). Every patched word is checked against the stock value first. Nothing in
the bootloader or the user page changes, so DFU recovery stays available.

Flash: `zd1 dfu write out.hex --backup <backup.tar> --yes --start` (`zd1 dfu backup` first) (DFU: hold ON/OFF at power-up).
Back to stock: the same with `blobs/zd1/mcu/zd1-2.04.05.hex`.

The hook blob (`telnet_hook.bin`) is committed; `build.sh` rebuilds it from the source (`nix build .#telnet-hook`)
and shows a diff if anything changed.

## Dumping BL2 from a ZD1L / A7

The point of the hook on a stock ZD1L or A7 is a shell on a unit we don't have, to read its BL2 out — the one
build of the module's updater that differs from ours (`docs/module/boot-chain.md`, "BL2"; the Lightning model's is
not ours, `docs/mcu/stock-firmware.md`). BL2 lives in NAND, so the running stock module app reaches it through its
own NAND driver with `fburn`; the hook just puts that shell on the network. Reading never writes flash, so no
interlock and no way-back concern — `fburn rd` only streams bytes out.

Steps (all on the owner's own unit, with their OK):

1. **Patch and flash the MCU.** `python3 -I stock-patch/patch.py --fw <model>.fw out.fw` (ZD1L `blobs/zd1l/zd1lv20435.fw`, A7
   `a7v20477.fw`), then upload `out.fw` **whole** to the module's **BL2 web updater**: key-hold ON/OFF while plugging
   mains in → the module comes up on wired Ethernet (DHCP) serving the firmware-update page → upload `out.fw`. BL2
   burns both parts (module `.bcd` to NAND, the patched MCU `.dmb` to the MCU over SPI) and resets. Proven on a real
   ZD1L [HW 2026-11-16] (`docs/module/boot-chain.md`, "The ZD1L BL2"). `python3 tools/zeppelin bl2web IP out.fw
   --yes` runs this BL2's upload sequence (the apply page, then the expert confirm). On our own ZD1, by contrast, the
   MCU route is DFU of the `.hex` (`zd1 dfu write`); the ZD1L/A7 ship the MCU app as a `.dmb`, which is what BL2's
   SPI path consumes.

2. **Reach the module and confirm the shell.** With the patched MCU up and the module ready, telnet port 23 is open
   on the module's IP (the hook re-asserts reg 0x75/0x76 every ~30 s). Point the tools at it with `ZD1_IP` (or
   `--host IP`); on a wired USB-Ethernet dongle the module answers on its link-local `169.254.1.1` (the default).
   Check the shell answers:

   ```
   ZD1_IP=<module-ip> python3 tools/zeppelin knos "os timestamp"      # prints the uptime from sds://>
   ```

3. **Read BL2.** The BL2 partition starts at NAND (FlashId 0) `0xA0000`; its length is the build's — the ZD1L's is
   **`0x1A0000`** (confirmed on hardware), our ZD1's `0x160000`. Read the header first and size the dump from its
   `partitionLength` (next caveat), e.g. for a ZD1L:

   ```
   ZD1_IP=<module-ip> python3 tools/zeppelin fburn rd 0 0xA0000 0x1A0000 bl2-<model>.bin
   ```

   `fburn rd` CAN-flushes the shared shell first, waits for "Start xmodem now!", then receives XModem-CRC
   (128/1024-byte packets) and truncates to the requested length (`docs/module/knos.md`). The result is the
   partition as burned: a partition header (first `0x80` bytes: `char date[16]`, modelId, swVersion, partitionId
   = 1 for BL2, partitionLength, imageLength, …), then the boot header (`0xAAAA5555` at `+0x80`), stage 1 and
   stage 2 — the same layout as our `blobs/zd1/module/bl2.bin` (`docs/module/boot-chain.md`, "Partition header").
   Parse/verify it with `python3 tools/zeppelin bcd` on the carved image, and compare against `bl2.bin`
   (build 2010-09-27) to see how far the Lightning model's updater has diverged.

Notes and caveats:

- The partition **base** `0xA0000` holds across models, but the **length does not**: the ZD1L's BL2 is `0x1A0000`,
  our ZD1's `0x160000` — so don't assume one. Read the first block (`fburn rd 0 0xA0000 0x20000 hdr.bin`), take
  `partitionLength`/`imageLength` from its header (`char date[16]` at 0, then the u32s; `docs/module/boot-chain.md`,
  "Partition header"), and size the full read from that.
- BL1 (block 0, `0x0-0x20000`) and PersParam (`0x20000`) read the same way, but BL1 is encrypted
  (`docs/module/boot-chain.md`) and PersParam holds per-unit data (the MAC) — keep any PersParam or user-settings
  dump (`0x60000`) out of git.
- *Writing* BL2 back from a running app is a different, unsolved problem (`fburn ebin`'s confirm never reads over
  telnet: `docs/module/boot-chain.md`, "Writing NAND from the stock app") and needs `--allow-bl2`. Dumping does
  not touch it.
