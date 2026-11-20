# Security: what we defend, and how

A speaker, not a bank. Two outcomes matter: **nobody causes a fire** (the amp's supply is the fragile part of a
Zeppelin Air and runs hot) and **the speaker doesn't end up in a botnet**. Everything else (someone on the network
playing music, a denial of service) is a nuisance we keep small where it is cheap. Assume an attacker who writes a
bespoke exploit for this one device: that is cheap now, so "nobody would bother with a speaker" is not a defence.

## The model
- **The MCU is the safety controller.** Its firmware keeps the amp safe: the heat trips (ADC4, ADC5), the amp and
  supply fault inputs, the volume cap and the heat cap (`mcu/src/protect.c`), the amp's idle timeout. None of it can be
  switched off over the host link: `Prefs` takes only bounded values (`volume_cap` 1..71, `amp_idle_s` 60..1800),
  `Amp`, `Peek`/`Poke`, `Dfu` are USB-only behind the gate.
- **The module is assumed taken over sooner or later.** Linux 2.6.24.7 (2008, `docs/module/linux.md`), an ARMv5
  without execute-never, static binaries at fixed addresses, everything as root, C decoders and a 2009 Wi-Fi firmware
  fed by the network: a remote bug there is a full takeover, and its many local kernel bugs make dropping root
  pointless (an unprivileged process becomes root in one step). Root on the module reaches `/dev/mem`, the NAND
  (the slots, the vendor area) and the SPI link to the MCU.
- So the line that must hold is **module -> MCU**: whatever the module does, the MCU's firmware stays ours and its
  limits stay in force.

## What holds that line
- **No MCU firmware from the module without a person.** Both bootloaders that take an app from the module (our ZD1
  preloader, the ZD1L's B&W "Net HBL") check only a CRC, and both start only when our app sends the MCU there (or by
  hand, ON/OFF at power-up). Our app goes only after a person at the speaker allowed it, once, until 60 s after the module's next ready
  (10 minutes at most: an update that brings no app, or whose trial fails, leaves none open): the
  host link's `Allow`, which our OS asks at the start of a network update, takes ON/OFF held 5 s (`mcufw_allow`,
  `mcufw_request`, `docs/mcu/bootloader-hbl.md`), read from the key's pin (`inputs_confirm_take`): nothing on SPI, the
  host link or IR can fake it. Without an allowance the module's request is refused, and nothing waits.
- **The MCU's parsing of what the module sends is bounded**: every notification is read into a fixed buffer with its
  length kept (`dm870_reg_read_v`, `drain`), handlers check lengths before they index (`wifi.c`, `track`), strings are
  copied with their bound (`copy_str`); a network request through the tunnel goes through the same generated decoder
  as USB's and only if marked `net` (`hostif.c` `run`).
- **What a taken-over module can still do**: everything a paired network client can (play, the volume up to the cap,
  `Prefs` within its bounds, settings, its own system slots), and keep itself on the NAND across reboots (no secure
  boot on the module). It can't flash the MCU, lift the cap or the heat limits, or keep the amp on past them.
  Recovery: `zd1 update` over USB (a clean system), BL2 (`zd1 module esc`), JTAG.

## What keeps the module from being taken over
Nothing on the module is reachable from the internet: no UPnP, no IPv6 (no global address), the speaker sits behind
the router's NAT. What reaches it is the local network (and through it, a browser on that network), the radio, and
the answers to its own requests (DHCP, DNS, NTP, Spotify's servers over TLS).
- **The network link** (`zd1-netd`, `docs/protocol/net.md`): Noise with both ends' keys; an unpaired client gets
  `Hello` and `Pair` only; a system update must be signed by a key the image trusts and be no older than what runs.
  Pairing takes ON/OFF held 5 s (a press would pair whoever kept a `Pair` waiting when someone next switched the
  speaker on). Only our page's `Origin` is let in: any other site the person has open would otherwise reach the
  speaker from their browser, which is on the speaker's network. Both handshakes have one 10 s deadline and an
  unpaired session ends after 60 s, so nobody holds the four sessions without pairing.
- **The receivers** (AirPlay, Spotify, Snapcast) are Rust (`shairplay` has no `unsafe`); what they hand to C is the
  audio decoders (`module/codecs`: AAC from AirPlay senders, FLAC from a Snapcast server, Vorbis from Spotify),
  fuzzed under ASan/UBSan, coverage-guided (`check/libfuzz.c`) and with each one's own mutator (`check/*check.c
  fuzz`). The same C runs on Warp0, which has no MMU: a decoder bug there writes anywhere in SDRAM.
- **The debug services** (a root shell on telnet 2323, raw memory on 23) are off in every published image and switched
  only from USB (`zd1 debug`, RAM only); `os-debug.bcd` has them on by default and is for development, never published.

## Known and accepted
- The kernel's remote denials of service (TCP SACK slowness CVE-2019-11478, low MSS CVE-2019-11479, SYN+FIN
  CVE-2012-6638, socket backlog CVE-2010-4251/4805) and its MD4 sequence numbers (CVE-2011-3188): a nuisance from the
  local network; the 240 MHz ARM falls over to any flood anyway. The kernel has no IPv6, SCTP, DCCP, netfilter, PPP,
  tunnels or Bluetooth, which most remote CVEs of its age need; its 802.11 is in the Wi-Fi core's firmware (FullMAC),
  not in mac80211.
- The Wi-Fi core's firmware (2009, never updated): parses every frame in radio range before association and can DMA
  into SDRAM. Nothing to be done; its compromise is the module's (above).
- AirPlay without a PIN by default (as AirPlay speakers are): anyone on the network can play, under the MCU's caps.
  `zd1 settings airplay.pin` sets one.
- The first pairing trusts the speaker's key it meets at that address (as SSH does): someone already answering for the
  speaker's address on the network at that moment could stand in between, and the person's 5 s hold would pair them.
- Time from NTP unauthenticated: it only matters to TLS's certificate dates (Spotify); the update's age check uses the
  build date, not the clock.
