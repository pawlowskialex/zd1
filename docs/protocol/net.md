# The network link: the speaker from the page over the LAN

How the web page (https://pawlowskialex.github.io/zd1/) and `zd1` reach the speaker over the network as well as over
USB: control, settings and updates, for a Zeppelin Air with Lightning (ZD1L, whose first install is over the network)
and for both models once our OS runs. One protocol whichever the wire: the host link (`protocol/host`, `host.md`), its
messages carried by our module OS. Tags: `docs/README.md`.

## Goals
- **The same page, the same flows**: connect over USB or by address; what the network can do, the page does the same
  way (`host/core`, one implementation).
- **No weaker than USB**: a change to the speaker needs a person at it once (pairing, ON/OFF held 5 s) and the
  paired client's key from then on; nothing on the network is ever a root shell, raw memory, a flash of a bootloader or
  of the boot image.
- **Updates over the network**: the system into the slot that doesn't run, signed by a key the speaker trusts, started
  on trial; the MCU follows its OS.

## Reaching the speaker from an HTTPS page
A page served over HTTPS may reach a plain-HTTP address on the local network in Chromium with **Local Network Access**:
the first request asks the person ("allow this site to access devices on your local network", Chrome 142 on), and with
that granted the mixed-content block is lifted for private IP literals and `.local` names (no `targetAddressSpace`
needed for those); WebSockets from Chrome 154. No certificate can name a LAN address, so plain HTTP with that
permission is the way; WebUSB already limits the page to Chromium on the desktop. The page takes an address (an IP, or
the speaker's `NAME.local`: our OS answers mDNS for its host name); browsers can't browse mDNS, `zd1` can.

## The link
```
page / zd1 ── WebSocket ws://SPEAKER/zd1 ── Noise_XX_25519_ChaChaPoly_SHA256 ── host link messages
                                             zd1-netd (module) ── zd1-linkd ── SPI reg 0xD9 ── the MCU
```
1. **HTTP on port 80**, `GET /zd1` upgraded to a WebSocket (binary messages only). Anything else is 404 (the place for
   the home-automation API later).
2. **Noise_XX** (`snow`, prologue `zd1-net 1`): three handshake messages, one per WebSocket message (client, speaker,
   client), then each WebSocket message is one Noise transport message. Both ends have a static X25519 key: the
   speaker's is made at its first start and kept in `/data/zd1/net/key`; a client's is its own (the page's in the
   browser profile's IndexedDB, `zd1`'s in `~/.config/zd1/net-key`). The handshake proves both and gives keys that
   encrypt and authenticate everything after it, in order (replay or reordering breaks the session).
3. **Inside**: the host link's own framing. The client sends a call, `u16 seq` + a request message (`host.md`); the
   speaker sends records as POLL does over USB (kind, u16 length, payload): REPLY (`u16 seq` + the reply), TEXT, and
   EVENT (a message the speaker pushes: `State` whenever it changes). The client may have several calls out; replies
   come in their order.

The client remembers the speaker's static key with its address and refuses another one there ("not the speaker you
paired with": pair again, which needs the press). The page keeps its key and the speakers' in its storage (the
origin's: every page under the same GitHub Pages host could read them), `zd1` in `~/.config/zd1` (`net-key`,
`speakers`).

## Pairing
After the handshake the speaker knows the client's static key. A key it hasn't paired is answered `Hello` and `Pair`
only; anything else is `Error LOCKED` ("pair first"). **`Pair`** (with a name for the client) asks the MCU for ON/OFF
held 5 s (the LED flashes purple, 30 s): confirmed, the key is paired, kept in `/data/zd1/net/controllers` with its
name and date. One `Pair` at a time; a refused one holds the next back 10 s. The gate's state is the USB side's: a gate
opened for good opens nothing on the network, and a pairing opens nothing on USB.

Why a 5 s hold and not the USB gate's press: anyone on the network can ask to pair at any time and keep a `Pair`
waiting, so a press would pair whoever asked when someone next switched the speaker on or off. A short press and the
2 s hold (standby) work as always while a `Pair` waits; the confirmation is the key's pin held past that
(`mcu/src/inputs.h`), which nothing on the host link or the remote can fake. Pairing leaves the speaker in standby
(the 2 s hold on the way): a press wakes it.

**Which pages may open the link**: a browser sends the page's address (`Origin`) with the WebSocket's request, and
any site the person has open could otherwise reach the speaker from their browser, on the speaker's network, and ask
to pair. Only the published page and a page served from this computer (`localhost`, `127.0.0.1`: `tools/zeppelin web`)
are let in (`net::origin_allowed`); programs (`zd1`) send none. Another site gets HTTP 403. A fork that publishes the
page elsewhere adds its address there.

Pairings are a record and an action, like AirPlay's: `ModRecord Controllers` lists them (over USB too), the action
`net.forget` forgets one (its id) or `all`; a forgotten key's open sessions end. Back to the factory (`reset all`)
forgets them all; so does erasing `/data`.

## What travels, and who answers
Each request of the spec is marked `net` when a paired client may make it over the network. **Who answers** is by
the request: the MCU's requests go through `zd1-linkd` and the SPI tunnel to the MCU, which runs them as over USB but
past the gate (the pairing was the press) and only if marked `net` (the module checks, the MCU again); the module's
own (settings, records, `Pair`, `SystemUpdate`) are answered by our OS without the MCU. Over USB it is the mirror:
the MCU answers its own and relays the module's (the configuration channel). `Hello` over the network is the MCU's,
its request list cut to what this link serves.

| Over the network | Requests |
|---|---|
| paired clients (`net`) | `Hello`, `Info`, `Diag`, `BootState`, `ModRecord`, `GetSetting`, `SetSetting`, `Key`, `Select`, `Volume`, `Prefs`, `Pair`, `SystemUpdate` |
| never (USB only) | `Dfu`, `Reboot`, `Module` (power, BL2, hold, BL1's menu), `Peek`, `Poke`, `ModRead`, `ModWrite`, `XmFrame`, `Pipe`, `PipeData`, `Update` (the UART path), `Boot` (the slots: an update over the network starts its own trial), `Console`, `ConsoleWrite`, `Debug` (a root shell), `Amp` (the interlock stays at the speaker), `Lock`, `Unlock` (`Pair` is the network's) |

**The tunnel** (SPI reg 0xD9 `HOST`, `registers.md`): `zd1-linkd` notifies `u16 seq` + a request, the MCU writes
`u16 seq` + its reply back. One request at a time in the MCU, from either side: a network request waits while a USB
one runs and the other way round.

## Network settings changed over the network
A change that can cut the client off (`net::provisional`: any `eth.*` or `wifi.*` setting or action, `reset`) made over
the network is provisional (`zd1-netd`, guard.rs): before the first one the settings file and the known networks are
kept, the change is applied, and they come back (the services that use them started again) unless the same client
opens a new session within 2 minutes (`CONFIRM_S`), at the old address or a new one. Only a session opened 10 s
(`SETTLE_S`) or more after the last change counts: init restarts what uses a setting 1.5 s after a burst of changes,
and a session opened sooner may still go over the old settings (one did: `eth.mode off` "confirmed" before the
interface went down [EMU 2026-11-19]). One client's changes at a time: another client's provisional change gets
`Error BUSY` meanwhile. `zd1 --net` and the page connect again by themselves after such a change (`reachAgain`), the
page at the fixed address an address change set too.
[EMU 2026-11-19, `zemu run --mcu --fwd 18080:80` with `zd1 --net`: `eth.dns` kept once the client came back; `eth.mode
off` cut the emulator's only interface, the client couldn't come back, and 120 s later the speaker went back to DHCP
and answered again. The emulator's clock runs ~3x faster than the wall: the client's window (wall time) outlived the
speaker's, and it reported "kept" for the change the speaker had undone; on hardware the client's ends 10 s earlier.]
[HW 2026-11-19, the unit on Wi-Fi, `zd1 --net`: `eth.dns` set, "the change is kept" 11 s later, still set past
the 120 s; cleared the same way. A change that cuts the link not tried on the unit.]

## System updates over the network
`SystemUpdate` (`host.md`), answered by our OS:
1. **BEGIN** carries the signed header (`protocol/release`): the image's version, build date, length, SHA-256, an
   Ed25519 signature over all of it. Refused unless a trusted key signed it (`/etc/zd1/keys/*.pub` in the image: the
   release key, `module/os/rootfs/etc/zd1/keys/release.pub`, whose secret half CI signs with; `/data/zd1/keys/*.pub`:
   a developer's, put there over USB, `zd1 console run 'echo KEY > /data/zd1/keys/NAME.pub'`), unless it is at least
   as new as the running system (its build date: an older signed release can't be replayed onto the speaker; going
   back is the USB updater's), while another update runs (USB or network: one lock, `zd1_slot::update_lock`), or
   while the running slot is on trial (the good slot is the way back). Then a person at the speaker allows it: our OS
   asks the MCU's `Allow` (ON/OFF held 5 s within 30 s, the LED flashing amber; the client says so), which allows the
   MCU firmware the system brings too; nobody: `Error DENIED`, nothing written [HW 2026-11-20: the refusal]. [EMU
   2026-11-20, `zemu run --nand --mcu --fwd 18080:80 --confirm 15 --host 'zd1 --net 127.0.0.1:18080 update ...'`: the
   hold, "allowed: an app from the module, once, ...", the update into slot B, its trial, then at the
   MCU's hello the system's MCU app asked for and taken at once: the preloader, "app in flash and checked"; again with
   the allowance ending 60 s after that ready, 74061cb.] [HW 2026-11-20: after a system-only update the allowance
   ended 60 s after the trial's ready, and an app offered then was refused.]
2. **DATA**, the image in order, up to 32 KB a request: unpacked into the slot that doesn't run as it comes
   (`zd1-slot`, as `zd1-update` does), each reply the bytes taken.
3. **END**: the stream's end checked (the gzip CRC, the SHA-256 of the header), the slot stamped `VERSION SHA[:12]
   LENGTH`, the reply DONE with the slot; then our OS asks the MCU to start it once (SPI reg 0xD8 `TRY`): ready makes it
   the good slot, no ready in 90 s and the MCU goes back. The client connects again and reads the State's `slot`,
   `good` and `os`.

The boot image (stub, kernel, rescue) is never written over the network: the vendor partition is read-only to Linux.
A release whose boot image differs (State's `boot`) says so and needs USB (the ZD1's relay) or BL2's web updater (the
ZD1L).

Signing: `zd1 keygen KEY` (the secret half, and `KEY.pub`), `zd1 sign system.cpio.gz --key KEY` (a `.signed` file:
the header, then the image), `zd1 --net HOST update [IMAGE] [--key KEY]` (signs on the way if given a plain image). A
release gets `system-VERSION.signed` beside its other images when the repository has the secret `ZD1_RELEASE_KEY`
(`.github/workflows/pages.yml`); `python3 tools/zeppelin web --key KEY` offers this checkout's build signed. Over USB
`zd1 update` and the page take a signed image too (its header dropped: the USB path takes any image).
[EMU 2026-11-19, `zemu run --nand --mcu --fwd 18080:80`, slot A holding this build: `zd1 --net update` of it signed
with a key in `/data/zd1/keys` went into slot B (5.2 MB at ~131 KB/s through the emulator's network), slot B started
once, got ready, the good slot; one signed by an untrusted key refused at BEGIN; one with a byte flipped in its image
failed unpacking, slot B left without a system. The page's WebAssembly did the same steps but the update in node
(`host/web/test/net.mjs`).]
[HW 2026-11-19, the unit on Wi-Fi with system 6512879 in slot A: `zd1 --net update --key` of the same build, the
public half in `/data/zd1/keys`: into slot B, started once, the good slot, 95 s in all (36 s of it the start); one
signed by another key refused at BEGIN ("signed by none of the keys this speaker trusts"), the slots untouched.]

**The MCU follows its OS**: each system carries its MCU app (`/usr/lib/zd1/mcu/zd1.dmb`, the same build's: `nix
build .#os-image` takes `.#mcu`); at the MCU's hello (its version, then the good slot), on a confirmed slot (not the
rescue, not on trial), with the MCU's version not the app's and neither a dev build (`-dirty`), `zd1-linkd` stages it
and asks the MCU (`docs/mcu/bootloader-hbl.md`). The MCU takes it only until 60 s after the module's next ready (10
minutes at most) after a person at the
speaker allowed it, once: the network update's start asks for that (BEGIN, `Allow`), so the hold that allows a system
also allows the MCU firmware it brings, and nothing waits afterwards. The MCU is what keeps the amp safe, and a module
someone took over must not be able to give it a firmware without them. Without an allowance the request is refused
(the MCU logs it); a version the bootloader took twice and the MCU doesn't run isn't sent again
(`/data/zd1/mcu/follow`: VERSION TRIES).
[HW 2026-11-19: the MCU on 249a535, the system 6512879 on its confirmed slot: asked at the ready, the MCU through our
preloader, back as 6512879 45 s later (`docs/mcu/bootloader-hbl.md`).]

## What each update takes from a release
A release's parts are one build of its tag (`.github/workflows/pages.yml`, `docs/web.md`, Releases); the MCU's app in
the system (`usr/lib/zd1/mcu/zd1.dmb`) is the same build's `.#mcu`, byte for byte, so the system's signature covers it.

| From | Takes | The MCU |
|---|---|---|
| our OS, over the network | `system-V.signed`, after ON/OFF held 5 s on the speaker | its app from the new system, under the same hold; only when its version (the app's content) differs |
| any of our releases or the stock firmware, over USB (the page's Update, `zd1`) | `os-V.bcd` when the stub or kernel differ, and the system from it (its rescue installs itself); else `system-V.cpio.gz`; `zd1-V.hex` | over DFU, the whole region: the preloader too |
| a ZD1L, its first install (key-hold, BL2) | `zd1l-V.fw` | B&W's bootloader takes the `.fw`'s app from BL2 |

So a ZD1 on a release from before the network link (no `zd1-netd`) takes one update over USB; from then on releases
go over the network, unless one changes the stub or the kernel, or the preloader, which only DFU writes. Over USB with
a boot image the page does what a first install does (host/core's `install`): the boot image and the MCU firmware in
one visit to the bootloader, our firmware started once, then `Boot INSTALL`: the rescue copies itself into the slot
that isn't the good one and the MCU starts that slot once (`docs/module/storage.md`, The rescue).

## Limits
Four sessions at a time (a fifth connection gets HTTP 503); both handshakes (the WebSocket's, Noise's) within 10 s
in all; an unpaired client's session ends 60 s after it began (its `Hello` and a `Pair`; nobody holds the sessions
without pairing); a WebSocket message at most 40 KB; a session idle for 5 minutes ends; one update at a time. `zd1-netd` runs as init's service `net`; the setting `net`
(`off`: nothing listens) is written over USB only.

## The ZD1L's first install from the page
A stock ZD1L held in BL2 (its key-hold: the light goes white, BL2 on wired Ethernet with DHCP; `docs/module/boot-chain.md`,
The ZD1L BL2) takes `zd1l.fw` through BL2's web updater (`host/web/www/js/zd1l.js`). BL2 sends no CORS headers, so
the page sends `bl2web`'s sequence (`tools/zeppelin/knos/bl2web.py`, which ZD1L owners ran) without reading the
answers (`no-cors`): the status page (only a network error says nothing is there), prepare, the upload (its 302 to the
apply page, which the browser follows: it arms the confirm), the expert page, the expert confirm. Then it waits up to
15 minutes for our OS's link to answer at the address and connects. A refused file (another model) or a new address
shows only as no answer. Releases carry `zd1l-VERSION.fw` (`kind` fw in the manifest). The requests are `bl2web`'s,
field for field. [EMU 2026-11-20, a stock ZD1L (its BL2 and HWID, KnOS, B&W's bootloader and app) held at power-up,
`bl2Upload` cut out of `zd1l.js` and run in node against BL2: every request answered 200, the upload's redirect to the
apply page followed, the confirm redirected to the status page (armed); BL2 sent the MCU part to B&W's bootloader,
burned the module image, the bootloader started our app, the rescue filled slot A and started it, and the page's
WebAssembly (`test/net.mjs`) paired with the hold on slot A: 374 s in all. `docs/mcu/bootloader-hbl.md`.] Never from
a browser against a real BL2 yet.

## Where things are
- The spec: `protocol/host/src/lib.rs` (the `net` marker, `Pair`, `SystemUpdate`, `Controllers`; `net`: the
  constants, the key file `KeyPair`, `provisional`; `record`: encoding and splitting the records);
  `protocol/spi/src/spec.rs` (`HOST` 0xD9, `TRY` 0xD8); `protocol/release` (the signed header, the `.dmb`, the
  firmware's identification).
- The module: `module/os/apps/zd1-netd` (the WebSocket, Noise, pairings, `SystemUpdate`), `zd1-linkd` (the tunnel,
  the module's answers, `try`).
- The MCU: `mcu/src/hostif.c` (the tunnel's requests), `mcu/src/dm870.c` (0xD8, 0xD9).
- The computer: `host/core` (`link.rs`: a link over USB or the network, `Mcu` over either; `net.rs`: the session;
  `update.rs`: `system_signed`, `system_over_network`, `trial`; `image.rs`: `sign_system`, `signed_system`),
  `host/cli` (`--net`, `net.rs`: the TCP WebSocket and `~/.config/zd1`; `sign`, `keygen`, `paired`), `host/web`
  (`net.rs`: the browser's WebSocket; `Speaker.connectNet`, `reachAgain`; www: `connection.js` `connectNet`,
  `zd1l.js`; `test/net.mjs`).
- The emulator: `zemu run --mcu --fwd 18080:80 --confirm 5 --host '...'` (`--confirm`: ON/OFF held 5 s, 5 s
  later, for the pairing; `emu/README.md`).

## State
On the unit [HW 2026-11-19, MCU and system 6512879, over Wi-Fi]: `zd1 --net` paired with a press, Info, the volume,
the settings and the slots through the tunnel and our OS, Peek refused (by `zd1`: the Hello masks what doesn't
travel), a provisional setting kept, the signed update into the other slot with its trial, an untrusted signature
refused, the MCU following its OS through the preloader. In the emulator only [EMU]: the page's WebAssembly over the network (in node), a provisional change that
cuts the link going back, the refusals of the module and the MCU themselves (an unpaired client, another key at an
address, a request not marked `net` sent anyway). Not yet anywhere: Chrome's Local Network Access from the hosted
page (the permission, `ws://` from HTTPS), the release key (the owner makes it: `zd1 keygen`, the secret in the
repository, the public half in the image), the ZD1L's install from the page (`docs/backlog.md`, Updates over the network).
