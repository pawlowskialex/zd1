// The Lightning model through its updater (BL2's web updater, from key-hold; docs/protocol/net.md, The ZD1L's first
// install): the release's .fw for our software, or B&W's own .fw to go back, with host/core's bl2web
// (js/bl2.js; `zd1 bl2web`; the same requests as the Python tool seen working on ZD1Ls). In a browser the updater's answers can't be read
// (no CORS headers): the file goes blind, and the page waits for our software to answer at the address. Where they
// can (zd1-app), the page says what answers there before anything is sent, sees a refusal, and follows the write; and
// it can look for the updater on this computer's networks.
import { newIdentity, Speaker } from "../pkg/zd1_web.js";
import { bl2Burn, bl2Look, bl2Upload } from "./bl2.js";
import { connectNet, events, job } from "./connection.js";
import { host } from "./host.js";
import { fetchImage, releases } from "./software.js";
import { $, ask, notice } from "./ui.js";

/// Our software at `addr`: its network link answers the session's handshake.
export async function ours(addr) {
  try {
    let me = localStorage.getItem("zd1.identity");
    if (!me) {
      me = newIdentity();
      localStorage.setItem("zd1.identity", me);
    }
    (await Speaker.connectNet(addr, me, undefined, events)).free();
    return true;
  } catch {
    return false;
  }
}

/// What the page can say of the updater at `addr` (null: nothing to say before it is tried).
async function look(addr) {
  try {
    const b = await bl2Look(addr, { timeoutMs: 4000 });
    if (b.kind === "zd1l") {
      // what it has installed, where it says (a main board it hasn't asked says 0)
      const has = [
        b.mcu && b.mcu !== "0" ? `main board ${b.mcu}` : "",
        b.module && b.module !== "0" ? `module ${b.module}` : "",
      ].filter(Boolean);
      return {
        ok: true,
        text: `Its updater answers at ${addr}: ${b.model || "the Lightning model"}${has.length ? `, ${has.join(", ")}` : ""}.`,
      };
    }
    if (b.kind === "zd1")
      return {
        ok: false,
        text:
          `At ${addr} answers the updater of a Zeppelin Air with the 30-pin dock: that one installs over USB, from ` +
          "its bootloader (the card beside).",
      };
    if (b.kind === "other") return { ok: false, text: `Something answers at ${addr}, but not the speaker's updater.` };
    return null;
  } catch {
    return {
      ok: false,
      text:
        `Nothing answers at ${addr}: is the speaker's light white (its updater), its cable in, and the address the ` +
        `one your router gave it?${host.lan ? "" : " The browser may also have been refused access to the local network (it asks once)."}`,
    };
  }
}

/// The address field looked at (zd1-app: the updater's answers can be read), its line under the field.
export async function lookHere() {
  const addr = $("zd1l-addr").value.trim();
  const line = $("zd1l-seen");
  if (!host.lan || !addr) {
    line.hidden = true;
    return;
  }
  line.hidden = false;
  line.className = "small";
  line.textContent = `Looking at ${addr}…`;
  const l = await look(addr);
  if ($("zd1l-addr").value.trim() !== addr) return; // typed on meanwhile
  line.textContent = l?.text ?? "";
  line.className = l?.ok ? "small ok" : "small warn";
}

/// Each address of this computer's networks (zd1-app), nearest first: 169.254.1.1 (BL2's own address on a cable to
/// this computer, no DHCP server), then the networks' neighbours (up to /22 each).
async function candidates() {
  const nets = (await host.networks?.()) ?? [];
  const out = ["169.254.1.1"];
  const ip = (s) => s.split(".").reduce((n, b) => n * 256 + +b, 0);
  const str = (n) => [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].join(".");
  for (const n of nets) {
    if (n.prefix < 22 || n.address.startsWith("169.254.")) continue;
    const me = ip(n.address),
      size = 2 ** (32 - n.prefix),
      base = me - (me % size);
    for (let i = 1; i < size - 1; i++) if (base + i !== me) out.push(str(base + i));
  }
  return [...new Set(out)];
}

/// The Lightning model's updater on this computer's networks (zd1-app): its address in the field, or a notice.
export async function findUpdater() {
  const b = $("zd1l-find");
  b.disabled = true;
  const line = $("zd1l-seen");
  line.hidden = false;
  line.className = "small";
  const all = await candidates();
  let next = 0,
    done = 0,
    found = null;
  const worker = async () => {
    while (!found && next < all.length) {
      const addr = all[next++];
      try {
        if ((await bl2Look(addr, { timeoutMs: 3500 })).kind === "zd1l") found = addr;
      } catch {}
      line.textContent = `Looking for its updater on this computer's networks: ${++done} of ${all.length} addresses…`;
    }
  };
  await Promise.all(Array.from({ length: 32 }, worker));
  b.disabled = false;
  if (!found) {
    line.className = "small warn";
    line.textContent =
      "No updater answers on this computer's networks. Is the speaker's light white, and its Ethernet cable in the " +
      "router this computer is on (or straight into this computer)?";
    return;
  }
  $("zd1l-addr").value = found;
  await lookHere();
}

/// Our release's .fw (`file` null), or a .fw from a file (B&W's own, to go back), through the updater at `addr`.
export async function installZd1l(addr, file = null) {
  addr = addr.trim();
  if (!addr) return notice("Its address first: the field above, or Find it.", "warn");
  const rel = file ? null : releases.find((r) => r.fw);
  if (!file && !rel) return notice("No release offers the Lightning model's file to this page.", "warn");
  const seen = host.lan ? await look(addr) : null;
  if (seen && !seen.ok) return notice(seen.text, "warn");
  const what = file ? file.name : `release ${rel.version}`;
  if (
    !(await ask(
      file ? `Install ${file.name} on the Lightning model?` : `Install ${rel.version} on the Lightning model?`,
      `${seen ? `${seen.text} ` : ""}${
        file
          ? `The file goes to the speaker's updater at ${addr}, which checks it and installs it on both chips (B&W's ` +
            "own file takes the speaker back to B&W's software)."
          : "An alpha: this release has run on a Lightning model only in the emulator. The release's file goes to " +
            `the speaker's updater at ${addr}, which installs it on both of its chips and restarts it with our ` +
            "software. If it doesn't come up, B&W's own file goes back on the same way."
      } About ten minutes: keep its power and its cable in meanwhile.`,
      "Install",
    ))
  )
    return;
  const done = await job(`The Lightning model: ${what}`, false, async () => {
    events.phase(1, 3, file ? "the file" : "the release's file");
    const fw = file ? file.data : await fetchImage(rel.fw);
    return writeFw(addr, fw, file ? file.name : rel.fw.file.split("/").pop(), { own: !file, phase: 2, of: 3 });
  });
  if (done && !file) await connectNet(addr);
}

/// A .fw through the updater at `addr`, within a job (its phases from `phase` of `of`: the upload, the write): then,
/// for ours (`own`), until our software answers at that address. The job's outcome.
export async function writeFw(addr, fw, name, { own, phase, of }) {
  const file = !own;
  events.phase(phase, of, `to the speaker's updater at ${addr}`);
  let read;
  try {
    read = await bl2Upload(addr, fw, name);
  } catch (e) {
    if (e?.code === "DEVICE") throw new Error((await look(addr))?.text ?? `Nothing answers at ${addr}.`);
    throw e;
  }
  if (read) {
    events.phase(phase + 1, of, "the updater writes it: the main board's part, then the module's");
    // its parts in the order it writes them [EMU 2026-11-21]: the main board's, then the module's
    const PARTS = { 1: "the main board's firmware", 2: "the module's software" };
    await bl2Burn(addr, ({ part, percent }) => {
      events.step(`writing ${PARTS[part] ?? `part ${part}`}${percent !== null ? `: ${percent} %` : ""}`);
      if (percent !== null) events.progress("write", (Math.max(+part || 1, 1) - 1) * 100 + percent, 200);
    });
    if (file)
      return "Written. The speaker restarts into it (if its light stays white, unplug its power and plug it in again).";
    events.step("written: the speaker restarts into our software");
  } else if (file) {
    return (
      `Sent to the updater at ${addr}: it installs the file now (its light white meanwhile), then the speaker ` +
      "restarts into it. This page can't see the updater's progress."
    );
  }
  if (!read) events.phase(phase + 1, of, "the speaker installs it and starts");
  const t0 = Date.now();
  while (Date.now() - t0 < 15 * 60 * 1000) {
    events.wait("our software answering", Math.round((Date.now() - t0) / 1000));
    await new Promise((r) => setTimeout(r, 10000));
    if (await ours(addr)) return `Installed: our software answers at ${addr}.`;
  }
  throw new Error(
    `Nothing of ours answered at ${addr} within 15 minutes. ${
      read
        ? "If the light is still white, unplug its power and plug it in again"
        : "If the light is still white, the updater refused the file (it checks the model)"
    }; if it plays, it may have taken another address (the router's list).`,
  );
}
