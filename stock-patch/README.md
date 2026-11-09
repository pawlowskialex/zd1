# Patching the stock MCU firmware

`patch.py` puts the telnet hook into the stock Zeppelin Air MCU app (2.04.05, `blobs/mcu/zd1-2.04.05.hex`): once the
module is ready, the MCU writes reg 0x75 = "23" and reg 0x76 = 5, the module's KnOS shell on telnet port 23,
re-asserted every ~30 s (`telnet_hook.S`). Python only:

```
python3 -I stock-patch/patch.py out.hex                       # ZD1 2.04.05 -> a .hex for DFU
python3 -I stock-patch/patch.py --fw zd1lv20435.fw out.fw     # ZD1L 2.04.35 -> a repacked B&W .fw
```
Flashed and checked on the unit 2026-10-16 (2.04.05). It is not on the unit now: that runs our own firmware (`mcu/`),
which has the amp interlock built in and puts the shell on telnet with `zd1 reg write` (`docs/module/knos.md`).

## The Lightning model (ZD1L 2.04.35)
The same hook ports to 2.04.35 with no change to the blob: every address it uses (`voTask` 0x8000e19c, `WRITEREG`
0x8000b910, the RAM anchors 0x3ee0/0x4df4/0x8250) is byte-identical, and the one patch literal that moved — sbrk's
heap base, 0x8002ae28 -> 0x8002ae24 with the -4 data shift — `patch()` locates instead of hardcoding. `--fw` patches
the MCU `.dmb` inside a B&W over-the-air `.fw` (`tools/zeppelin/fw/bwfw.py`) and repacks it, the module `.bcd`
untouched; the `.dmb` links at 0x80008000, so there is room for the hook at the cave. The anchors are asserted, so a
build that moved them (the A7's 2.04.77) fails rather than flashing wrong.

A repacked `.fw` gave the KnOS shell on telnet (`sds://>`, `help`) in the emulator, booting a stock Lightning module
image beside the patched MCU [EMU 2026-11-08: `zemu run module.bcd --mcu FLASH --renode 'zd1:board Adc "2:0x300"'`,
AUX boot source]. Delivery is the open part: what on a ZD1L takes a `.fw`'s MCU part is unconfirmed
(`docs/mcu/stock-firmware.md`), so this is the payload for that route, not proof the route exists.

How: the BcoWrapper tick calls `voTask` through a pointer literal at 0x8001e870; the patch points it at the hook in
free flash at 0x80038100, which runs a small state machine and then calls the real `voTask`. One byte of state at RAM
0x8250 (crt0's bss end and the heap base move from 0x8250 to 0x8258). Every patched word is checked against the stock
value first. Nothing in the bootloader or the user page changes, so DFU recovery stays available.

Flash: `zd1 dfu write out.hex --backup <backup.tar> --yes --start` (`zd1 dfu backup` first) (DFU: hold ON/OFF at power-up).
Back to stock: the same with `blobs/mcu/zd1-2.04.05.hex`.

The hook blob (`telnet_hook.bin`) is committed; `build.sh` rebuilds it from the source (`nix build .#telnet-hook`)
and shows a diff if anything changed.
