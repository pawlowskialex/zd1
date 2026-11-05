# DM870 Wi-Fi in the stock app (KnOS)

How the stock app drives the on-chip WLAN MAC ("WILD" core, codename "Eagle"; external RF T6201), from the app's
decompilation and the live shell (booted on Wi-Fi). Our OS uses the Nokia kernel's `sulu_wlan.ko` instead
(`re/netmodule/NOKIA_HD1.md` section 5; on the unit: `docs/module/memory-map.md`, Wi-Fi); this is the reference for what the hardware does
under KnOS. Tags: `docs/README.md`. No key material was captured.

## Shape
- The WILD core has its own ARM7 and firmware (version 0x00A70506 [HW]). Under KnOS the host moves Ethernet frames
  (`wlan_pkt_receive` below); where the 802.11 framing happens under KnOS was not traced. KnOS's firmware is not
  Nokia's: see The firmware.
- Configuration and status live in a **shared-memory MIB** that the host reads and writes directly.
- Frames move through the shared SuluDMA engine (bus master "WILD").
- The module binds one network interface at boot and does not fail over: booted on Ethernet, the WLAN device
  object is null (`*0x60901acc` = 0, "No wlan device available") and the MIB can't be read [HW].

## The MIB
28 groups, addressed as `(mibidx, elemidx, extraidx)`, mibid = `(group << 8) | elem`. Live dump:
`re/netmodule/shell-dumps/wlan_mib_show_wifi.txt`; the full schema (329 elements, names from the live dump):
`re/netmodule/wlan_mib_schema.txt`.

| idx | group | role |
|---|---|---|
| 00 APP_STAT | firmware/version/MAC-addr/last-error | 02 FirmwareVersion = 0x00a70506, 05 ModembVersion, 07 EagleVersion |
| 01 HD_STAT | host-driver TX/RX counters (TxOKCnt, TxErrorCnt, RxOKCnt, NoBufferCnt, wlan_reset_count) |
| 02 ME_CTE | MAC-entity constants |
| **03 ME_CONF** | **association config**: 01 DesiredBssid, 02 DesiredSsid, 03 DesiredPhyType (6 = 11g), 04 DesiredChannel, 05 DesiredRates, 09 Infrastructure (1 = BSS) |
| 04 ME_STAT / 05 ME_SP_CONF | MAC-entity status / supplicant config |
| 06 MM_CONF / 07 MM_STAT | MAC management: 07:07 WpaRequested, 07:08 WpaFrame (RSN IE) |
| 08 MM_ACL_CONF | ACL |
| 09 DP_STA_STAT / 10 DP_STA_PEER_STAT | datapath station/peer stats (rates, current channel 19:17) |
| 11 DP_MDS_CONF / 12 DP_MDS_STAT | datapath MAC-data-service (TX/RX path config) |
| **13 DP_SEC_CONF** | **security/keys**: 00 WEPDefaultKeyValue[4], 02 WEPKeyMappingValue[32] (MAC_SEC_KEY table, where WPA2 PTK/GTK go), 03 DefaultKeyID, 04 BssPrivacy |
| 14 DP_SEC_STAT | RSNA stats |
| 15 DP_PP_CONF / 16 DP_PP_STAT | packet-processing (offload) config/stat |
| 17 MB_CONF / 18 MB_STAT | mailbox |
| 19 TRC_CONF / 20 TRC_STAT | current PHY (17 CurrentChannel, 18 OperationalRates, 24 CurrentBssid) |
| 21 DBG_STAT / 22 DR_PHY_STAT | debug / PHY stats |
| 23 DP_WDS_CONF | WDS |
| 24 PS_STA_CONF / 25 PS_STA_STAT / 26 PS_APIBSS_STAT | power save |
| 27 HI_MII_STAT | host-interface (MII) stats |

Element types: U8/U16/U32, MAC_ADDR, MAC_SSID, MAC_RATESET, MAC_SEC_KEY, CO_BOOLEAN, …; each has an access mask
(r/w/l/v).

**Layout** [RE]: a schema table at 0x605c1f08 holds per group `{u8 count, u32 descPtr}` (8 bytes per group); each
`descPtr` points to `count` × 6-byte element descriptors `{u16 offset, u8 size, u8 flags, u8 maxcount, u8 _}`. The
per-group data base is `*(wlanDev + 6 + group·4)` with `wlanDev = *0x60901acc`; element address = group base +
offset (+ idx·size). Flags: bit 1 array, bit 2 needs the lock (FUN_60540a28/a60), bit 5 key-mapping table. A set
(`FUN_605117a0(wlanDev+6, mib, elem, extra, &value)` → `FUN_60530cfc` → `FUN_60533bac`) writes the value straight
into that memory; error codes 0x605 size too big, 0x606..0x609 other failures. A get formats it
(`FUN_603b4140(wlanDev+6, mib, elem, extra, outBuf, len)`; returned "0x00a70506/10945798" for 0/2/0 on the unit
[HW]). Asynchronous confirms and indications from the MAC (`ME_GetMib{Ok,Fail}Cfm`, `ME_SetMibFailCfm`,
`AssociateInd`, `DisassociateInd`) come through a separate path; MB_CONF (group 17) is the mailbox.

## Association
The live state shows the recipe (ME_CONF and the security group were set to reach "connected") [HW, INF for the
order]:
1. Scan: `sys wlan ctrl scan` / `wpa_driver_sulu_knos_perform_site_scan` → BSS list.
2. ME_CONF: DesiredSsid, DesiredBssid, DesiredChannel, DesiredRates, DesiredPhyType, Infrastructure.
3. Security: MM WpaRequested + WpaFrame (RSN IE), DP_SEC_CONF BssPrivacy, DefaultKeyID.
4. Keys: MAC_SEC_KEY entries in DP_SEC_CONF (WEPKeyMappingValue[32] for the pairwise PTK and the group GTK): the
   hardware CCMP offload; the app has no software CCMP.
5. Connect; the MAC authenticates/associates and raises `AssociateInd` (→ `wpa_driver_sulu_notify_os`).
Driver entry points: `wpa_driver_sulu_init` 0x6047cb4c, `set_mode` 0x6047dff8, `site_scan` ~0x605b8218; classes
`SuluWlanDevice`, `WlControlService`/`SluWlanControl`; the `sys wlan ctrl` handler `SuluWlanDeviceCtrlCmd`.

## The firmware [RE]
Two builds carry the version word 0x00A70506, neither with public source. Both are plain ARM7 images (vector table at 0,
unencrypted, unsigned), built with ARM's compiler (its C library's messages at the end):
- `sulu-wlan.fw-rev78512` (Nokia's, what our OS loads): 60,304 bytes, the version literal at +0x1648.
- The stock app's: at `re/gen/module-app/image.bin` +0x5992CC, about 70 KB, the version literal at +0x1A68. About 20 % of
  Nokia's 16-byte windows occur in it (735 of 3,769), so it is a different build, possibly the same sources in part.

Nokia's build, from a Ghidra decompilation (function addresses are offsets in the file; nothing here seen on the unit):
- It knows the MIB's layout: `DP_SEC_CONF` as in the schema (default keys +0x03, their status +0x87, key mapping +0x8B,
  DefaultKeyID +0x4AB, privacy +0x4AC).
- **RX** (`FUN_000036a8`): the Protected bit alone selects decryption, whatever the frame's type. The key comes from the
  sender's station entry, or the group table when the frame is multicast or the station has no pairwise key; no key is
  error 0x302, dropped.
- **TX** (`FUN_00004930`): privacy on and Protected set selects a key the same way, again with no check of the type.
  Otherwise it **clears the Protected bit** and sends in the clear, so a frame the host already encrypted can't pass.
- **Crypto** (`FUN_000038a4`): a descriptor to a hardware engine: cipher (WEP, TKIP, CCMP), key length, the TID, the
  header's length, the key. Nothing marks a management frame.
- **Authentication frames**: one test of subtype 0xB0 (`FUN_0000854c`, a one-shot hook on the receive path). Nothing
  reads the algorithm number or the body.

## WPA3
Nothing has been tried on the unit; this is what the code above implies [INF].

SAE itself should pass the firmware: the frames are ordinary cleartext Authentication frames, built by the host, and
the firmware doesn't read their algorithm. The radio has no part in it, and CCMP-128 is what WPA3-Personal uses.

PMF is the part the hardware may not do. 802.11w changes CCMP for management frames (a Management bit in the nonce,
different masking of the frame control in the AAD), and this engine predates it, so protected unicast management frames
would probably fail their MIC in both directions. Test it before relying on it. The way around is a small firmware
patch, since nothing checks a signature: treat Protected management frames as "no cipher" in `FUN_000036a8`, and keep the
Protected bit in `FUN_00004930`'s no-cipher path, with CCMP for those frames in software on the host. BIP (broadcast
management frames) is cleartext with a trailer, host software with no firmware change.

The blocker is the host, a binary 2.6.24.7 kernel:
- `mac80211.ko`'s MLME sends Authentication frames itself and ignores transactions it doesn't expect; no external
  authentication, no nl80211. `wpa_supplicant`'s SAE needs a driver that lets it run the SME (Wireless Extensions don't).
- It has `ieee80211_monitor_start_xmit` and radiotap, so a monitor interface could inject and hear the SAE frames.
  Getting `mac80211` from there into its associate state needs kernel code: an out-of-tree module
  (`docs/module/linux.md`) or a patched `mac80211.ko`. The RSN element (AKM 8, MFPR) already goes through WEXT's generic IE.
- `wpa_supplicant` needs a driver backend of ours with the SME flags, and an ECC library: `CONFIG_TLS=internal` has none.
- `mac80211` here doesn't enforce PMF: without a patch it accepts unprotected deauthentication. Such a join has SAE's
  gains (no offline dictionary attack, forward secrecy) but not PMF's, though the AP believes otherwise.
- The cost of SAE's P-256 work on the 240 MHz ARM926 is unmeasured, and an AP's patience with a slow peer is finite.

The cheapest test that decides it: a monitor interface on the unit, a test program, and a Commit/Confirm exchange with a
WPA3-only AP. WPA3 transition networks already join with WPA2.

## Datapath [RE]
- **Netif** ops struct at 0x6051e150: `{ctx, name "WLAN", "802.11", 0x60526dd4 link up/down ("Network driver %d
  status connected/disconnected"), 0x60526c00 = wlan_pkt_receive (RX in), 0x60526bc0, 0x60526a98 (reconfig /
  reassoc), a stub}`.
- **RX**: `wlan_pkt_receive(dataPtr, len)`, called from the DMA RX handler: allocates a packet buffer
  (FUN_60149a4e(len+2)), requires len ≥ 0xE, copies the 14-byte Ethernet header to buf+0 and the payload to
  buf+0x10, sets the ethertype at buf+0x24, links a netif entry, enqueues to the IP stack (FUN_60298788) and signals
  (FUN_602a625c).
- **TX**: IP stack → netif output → **WlanTxQ** (4 access-category queues, each with an 800-byte buffer) → SuluDMA →
  MAC. The WLAN DMA device struct (reset in FUN_60530918) holds the 4 TX contexts of 0x40 bytes, RX/state fields at
  +0x1a4..+0x1e0 and two RX context objects; the completion event is **WlanDmaEvent**. Its AHB endpoints are
  0x30000000-class addresses, the WILD core's memory window (`FUN_60533ed8` returns 0x30000000).
- DMA configuration also goes through the MIB: DP_MDS_CONF (11) and DP_PP_CONF (15). The SuluDMA context format:
  `SULUDMA.md`. The WLAN contexts' live values (endpoint, flag bits) were never captured: that needs a Wi-Fi boot with
  traffic and `sys dma lookup`, `sys dma ctx <c>`.
