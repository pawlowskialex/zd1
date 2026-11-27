// BL2's web updater, as the page talks to it: host/core's bl2web (one client for zd1, zd1-app and this page) over the
// page's fetch. zd1-app's reads every answer, and each step is checked as zd1 checks it; a browser's, with no-cors (BL2
// sends no CORS headers), reads none, and the steps go blind, as B&W's own page sends them (docs/protocol/net.md, The
// ZD1L's first install). Node's tests read everything (mode "cors": host/web/test/zd1l.mjs).
import { bl2Burn as burn, bl2Look as look, bl2Upload as upload } from "../pkg/zd1_web.js";

/// What answers at `addr` (HOST or HOST:PORT): { kind: "zd1l", model, module, mcu } (the Lightning model's 2012 BL2,
/// what it has installed), { kind: "zd1" } (the Zeppelin Air's 2010 BL2), { kind: "other" }, { kind: "blind" }
/// (something answers that this page can't read). Throws when nothing answers (within `timeoutMs` a request, 0: none).
export const bl2Look = (addr, { mode = "no-cors", timeoutMs = 0 } = {}) => look(addr, mode, timeoutMs);

/// A file to the updater at `addr`, written once confirmed: a B&W .fw to the Lightning model's (`lightning`), a .bcd to
/// the Zeppelin Air's. Returns whether the answers were read (then the write started for sure: follow it with
/// `bl2Burn`); throws on a refusal it could read.
export const bl2Upload = (addr, data, name, { lightning = true, mode = "no-cors" } = {}) =>
  upload(addr, mode, lightning, data, name);

/// The write, on BL2's status page, until it ends: `onStatus({ part, percent })` on each change (the Lightning model's
/// parts: "1" the main board's, "2" the module's). Throws if it says it failed, or stops answering for long. Readable
/// answers only.
export const bl2Burn = (addr, onStatus = () => {}, { lightning = true, mode = "no-cors" } = {}) =>
  burn(addr, mode, lightning, onStatus);
