#!/usr/bin/env python3
"""Expect/send driver for the module console socket of `zemu run --keep` (127.0.0.1:4555): steps from $STEPS, one per line.
  usage: STEPS=$'expect:REGEX\nsend:TEXT\n...' expect.py HOST:PORT LOGFILE SECS
  steps: expect:REGEX  send:TEXT (+CR)  key:X (no CR, \\xNN escapes)  khold:HEX:SECS (flood a byte, e.g. BL1's 2 s
         serial window: 01 = BL1 menu, 02 = BL2)  xsend:FILE (XModem-1K/CRC upload, e.g. to the BL1/BL2 menus)
         sleep:N  run:CMD
For what zemu's own steps don't do: XModem uploads to the BL1/BL2 menus, holding a key through a boot window."""

import os
import re
import socket
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "tools"))
from zeppelin import xmodem  # noqa: E402

host, port = sys.argv[1].split(":")
log, secs = sys.argv[2], float(sys.argv[3])
end = time.time() + secs
s = socket.create_connection((host, int(port)))
s.setblocking(False)
pos, ok = 0, True


def drain():
    try:
        while s.recv(4096):
            pass
    except BlockingIOError:
        pass


def text():
    drain()
    try:
        return open(log, "rb").read().decode("latin-1")
    except FileNotFoundError:
        return ""


def getc(t):
    s.setblocking(True)
    s.settimeout(t)
    try:
        b = s.recv(1)
    except TimeoutError:
        b = b""
    s.setblocking(False)
    if os.environ.get("XDEBUG"):
        print(f"    {time.time() % 1000:8.3f} rx {b.hex() or '-'}", flush=True)
    return b


def xsend(data):
    drain()

    # Emulation quirk (-M dm870 UART): KnOS's XModem receiver only takes block 1 if it is already queued when the
    # receiver starts: sent after its 'C' (the textbook order) it is dropped, the receiver re-sends 'C', then CANs.
    # So block 1 goes out right away: put 'expect:to cancel]' before the xsend step; later 'C's are ignored.
    # (the banner's CR LF and the start 'C's are skipped as the answers come: zeppelin/xmodem.py)
    def sent(n, total):
        if os.environ.get("XDEBUG"):
            print(f"    {time.time() % 1000:8.3f} block {n}/{total} ACKed", flush=True)

    try:
        n = xmodem.send(s.sendall, getc, data, timeout=10, progress=sent)
    except OSError as e:
        print(f"  xsend: {e}")
        return False
    print(f"  xsend: {n} blocks sent")
    return True


for step in filter(None, os.environ.get("STEPS", "").split("\n")):
    kind, _, arg = step.partition(":")
    t0 = time.time()
    if kind == "expect":
        rx = re.compile(arg)
        while time.time() < end:
            m = rx.search(text(), pos)
            if m:
                pos = m.end()
                break
            time.sleep(0.2)
        else:
            print(f"TIMEOUT {step}")
            ok = False
            break
    elif kind == "khold":  # flood a byte for N s (arg "HEX:SECS"), for a boot-time serial window
        hx, _, secs = arg.partition(":")
        bb = bytes.fromhex(hx)
        e = time.time() + float(secs or 3)
        while time.time() < e:
            s.sendall(bb)
            time.sleep(0.02)
    elif kind in ("send", "key"):
        data = arg.encode().decode("unicode_escape").encode("latin-1") + (b"\r" if kind == "send" else b"")
        for b in data:  # one byte at a time: the KnOS UART RX has a tiny FIFO
            s.sendall(bytes([b]))
            time.sleep(0.02)
    elif kind == "xsend":  # XModem-1K/CRC upload of a file (receiver starts with 'C')
        ok = xsend(open(arg, "rb").read())
        if not ok:
            print(f"FAILED {step}")
            break
    elif kind == "sleep":
        time.sleep(float(arg))
    elif kind == "run":  # a host command (e.g. a telnet client on 127.0.0.1:2323)
        r = subprocess.run(arg, shell=True, capture_output=True, text=True)
        print("\n".join("  run: " + l for l in (r.stdout + r.stderr).strip().splitlines()[-15:]))
        if r.returncode:
            print(f"FAILED {step} (exit {r.returncode})")
            ok = False
            break
    print(f"ok {time.time() - t0:5.1f}s {step}", flush=True)
while ok and time.time() < end and "wait" in os.environ.get("TAIL", ""):
    time.sleep(1)
text()
sys.exit(0 if ok else 1)
