// BL2's web updater, as the page talks to it (docs/protocol/net.md, The ZD1L's first install; docs/module/boot-chain.md,
// The ZD1L BL2): no imports, so node runs it against the emulator's BL2 too (host/web/test/zd1l.mjs).
//
// BL2 sends no CORS headers. A browser lets the page send the requests but read none of the answers (no-cors: an
// opaque response): the steps go blind, as B&W's own page would send them. Where the answers can be read (zd1-app,
// node), each step is checked as tools/zeppelin/knos/bl2web.py checks it: which updater answers and what it has
// installed, the file taken or refused, the write started, then its progress to the end.

/// The confirm's button value (BL2's aspSetFirmwareUploadConfirmationHandler), form-encoded.
const OK = "++++++Ok++++++";

const tag = (xml, name) => xml.match(new RegExp(`<${name}>([\\s\\S]*?)</${name}>`))?.[1].trim() ?? "";
const text = (html) =>
  html
    .replace(/^﻿/, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
const at = (addr, path, init = {}) => fetch(`http://${addr}${path}`, { cache: "no-store", ...init });

/// What answers at `addr` (HOST or HOST:PORT): `{ kind: "zd1l", model, module, mcu }` (the Lightning model's 2012
/// BL2, and what it has installed), `{ kind: "zd1" }` (the 30-pin model's 2010 BL2), `{ kind: "other" }`, or
/// `{ kind: "blind" }` when something answers that the page can't read (a browser). Throws when nothing answers.
export async function bl2Look(addr, { mode = "no-cors", signal } = {}) {
  const r = await at(addr, "/1000/firmware_status.asp", { mode, signal });
  if (r.type === "opaque") return { kind: "blind" };
  const body = await r.text();
  if (r.status === 200 && body.includes("<Status>")) {
    const cur = tag(body, "current");
    return { kind: "zd1l", model: tag(body, "Model"), module: tag(cur, "version"), mcu: tag(cur, "host") };
  }
  const old = await at(addr, "/1000/bl_firmware_update.asp", { mode, signal }).catch(() => null);
  return (await old?.text())?.includes("appFirmware") ? { kind: "zd1" } : { kind: "other" };
}

/// A file (a B&W .fw: the module's .bcd and the MCU's .dmb) to the updater at `addr`, written once confirmed: prepare
/// (it arms the upload), the file (its 302 to the apply page, which fetch follows: that page arms the confirm, without
/// it the confirm burns nothing), the expert page, the expert confirm (it starts the write: the MCU's part first,
/// through the MCU's bootloader, then the module's). Returns whether the answers were read (then the write started
/// for sure: follow it with `bl2Burn`); throws on a refusal it could read.
export async function bl2Upload(addr, fw, name, { mode = "no-cors" } = {}) {
  const go = (path, init = {}) => at(addr, path, { mode, ...init });
  const seen = (await go("/1000/firmware_status.asp")).type !== "opaque"; // throws only when nothing answers
  await go("/1000/firmware_prepare.asp");
  const form = new FormData();
  form.append("appFirmware", new Blob([fw]), name);
  form.append("appFirmwareFile", "Upload");
  const up = await go("/goform/formPostHandler", { method: "POST", body: form });
  if (seen) {
    const page = new URL(up.url).pathname;
    const said = text(await up.text());
    if (page !== "/1000/firmware_apply.asp")
      throw new Error(`The updater didn't take the file (it went to ${page})${said ? `: ${said.slice(0, 200)}` : ""}.`);
    if (said)
      throw new Error(
        /_HWID/.test(said)
          ? "The updater refused the file: it is for other hardware than this speaker's. Nothing was written."
          : `The updater refused the file: ${said.slice(0, 200)}. Nothing was written.`,
      );
  }
  await go("/1000/firmware_expert.asp");
  const c = await go(`/goform/formExpertDownloadConfirmationHandler?button=${OK}`);
  if (seen && !new URL(c.url).pathname.endsWith("/firmware_status.asp"))
    throw new Error("The updater took the file but didn't start writing it: nothing was written. Try again.");
  return seen;
}

/// The write, from BL2's status page until it says it finished (the MCU's part, then the module's: `onStatus({ part,
/// percent })` on each change); throws if it says it failed, or stops answering for long. Readable answers only.
export async function bl2Burn(addr, onStatus = () => {}, { pollMs = 2000 } = {}) {
  let lost = 0,
    last = "";
  for (;;) {
    await new Promise((r) => setTimeout(r, pollMs));
    let body;
    try {
      body = await (await at(addr, "/1000/firmware_status.asp")).text();
      lost = 0;
    } catch {
      if (++lost * pollMs >= 60000)
        throw new Error(
          "The updater stopped answering during the write. The speaker may have restarted by itself: give it a few " +
            "minutes, then look at its light (white: still its updater; else it starts what was written).",
        );
      continue;
    }
    const percent = [...body.matchAll(/<progress>(\d+)<\/progress>/g)].map((m) => +m[1]);
    const now = { part: tag(body, "module"), percent: percent.length ? percent[percent.length - 1] : null };
    if (JSON.stringify(now) !== last) {
      last = JSON.stringify(now);
      onStatus(now);
    }
    if (tag(body, "finished") === "YES") {
      const module = tag(body, "success"),
        mcu = tag(body, "success1");
      if (module !== "YES" || !["YES", ""].includes(mcu))
        throw new Error(
          `The updater says the write failed (the module's part ${module || "not done"}, the main board's ` +
            `${mcu || "not done"}). The speaker stays in its updater: try again, or B&W's own file.`,
        );
      return;
    }
  }
}
