// The software: the bootloader's view (install, repair, roll back), the slots, the releases, installs from files.
import { imageInfo, planUpdate, Speaker } from "../pkg/zd1_web.js";
import { backups, drawBackups, restoreBackup } from "./backups.js";
import { bl2Look } from "./bl2.js";
import { act, connect, connectNet, events, job, show } from "./connection.js";
import { app } from "./shared.js";
import { modelOf, refresh } from "./speaker.js";
import { $, ask, el, failed, names, notice } from "./ui.js";
import { ours, writeFw } from "./zd1l.js";

// ---- the bootloader: the installer ------------------------------------------------------------------------------

export async function drawDfu() {
  let st;
  try {
    st = await app.bl.state();
  } catch (e) {
    return failed(e);
  }
  const last = st.boot.at(-1);
  const lastOk = last?.stage === "main loop";
  $("dfu-boot").textContent = last
    ? `our firmware's boot ${last.boot} reached “${last.stage}”${lastOk ? "" : ": it stopped there"}`
    : "none of our firmware logged: B&W's firmware, a new speaker, or an install erased the log";
  $("dfu-module").textContent = st.module;
  const rel = releases[0];
  const btn = (label, primary, f) => {
    const b = el("button", { textContent: label, className: primary ? "primary" : "" });
    b.onclick = f;
    return b;
  };
  const row = (...kids) => el("div", { className: "row" }, ...kids);
  const small = (textContent) => el("p", { className: "small", textContent });
  const noRelease = (what) =>
    el("p", { className: "warn", textContent: `No release is offered to this page: ${what}.` });
  let lead;
  const p = [];
  if (!last) {
    lead = "Ready for our firmware.";
    p.push(
      el(
        "ol",
        { className: "steps" },
        ...[
          "The module's boot image, the release's system in it: about 6 minutes.",
          "The main board's firmware, after a backup of what is there (a download, and a copy in this browser).",
          "The module installs its system into a slot and starts it: about a minute.",
        ].map((textContent) => el("li", { textContent })),
      ),
      small(
        "Keep the speaker plugged in and this tab in front. It ends without a network: the page shows where to add one.",
      ),
      rel?.mcu && rel?.system
        ? row(btn(`Install release ${rel.version}`, true, () => installAll(rel)))
        : noRelease("install from files under Advanced"),
    );
  } else if (lastOk) {
    lead = "Our firmware is installed, and waits here.";
    p.push(
      small("It was asked into the bootloader, or ON/OFF was held while it powered up."),
      row(
        btn("Start the firmware", true, startFirmware),
        rel?.mcu ? btn(`Install release ${rel.version}'s firmware`, false, () => installRelease(rel, "mcu")) : null,
      ),
    );
  } else {
    lead = `Our firmware's last start stopped early, at “${last.stage}”.`;
    p.push(
      small("Repair writes the firmware again (a backup first); a backup below puts back what was there before."),
      row(
        rel?.mcu
          ? btn(`Repair: release ${rel.version}'s firmware`, true, () => installRelease(rel, "mcu"))
          : noRelease("install from a file under Advanced"),
        btn("Start it anyway", false, startFirmware),
      ),
    );
  }
  $("dfu-lead").textContent = lead;
  $("dfu-primary").replaceChildren(...p);
  drawBackups($("dfu-backups"), restoreBackup);
  // the backups only when there are some (or the browser keeps none: it says why)
  backups()
    .then((l) => {
      $("dfu-kept").hidden = !l.length;
    })
    .catch(() => {
      $("dfu-kept").hidden = false;
    });
}

export async function startFirmware() {
  const r = await job("Starting the firmware", false, () =>
    app.bl.start().then((r) => (r.matches ? `${r.fw} runs.` : "It started.")),
  );
  if (!r) return;
  app.bl = null;
  await connect(false);
}

/// Our firmware on both boards from the bootloader (host/core's `install`), the release's three images.
export async function installAll(rel) {
  if (
    !(await ask(
      `Install release ${rel.version}?`,
      "Both boards get our firmware; what is on the main board is " +
        "backed up first (a download, and a copy in this browser). About 7 minutes; the browser may ask once for the firmware.",
      "Install",
    ))
  )
    return;
  const r = await job(`Install release ${rel.version}`, false, async () => {
    // with a boot image its rescue is the system (it installs itself); without one, the system image
    const bcd = rel.bcd ? await fetchImage(rel.bcd) : undefined;
    const [mcu, system] = [await fetchImage(rel.mcu), bcd ? undefined : await fetchImage(rel.system)];
    const r = await app.bl.install(bcd, rel.mcu.file, mcu, system);
    return `Done: MCU firmware ${r.fw}, system ${r.os} in slot ${r.slot.toUpperCase()}.`;
  });
  if (!r) return;
  app.bl = null;
  await connect(false);
  if (app.sp) document.querySelector("#known").scrollIntoView({ behavior: "smooth" });
}

export async function rollback() {
  if (
    !(await ask(
      "Back to B&W's software?",
      `${app.rbBcd.name} to the module, then ${app.rbMcu.name} to the main board (our firmware ` +
        "backed up first). About 7 minutes. Afterwards the speaker runs B&W's software, as from the factory, and this page " +
        "can't see it any more.",
      "Roll back",
    ))
  )
    return;
  const r = await job("Back to B&W's software", false, () =>
    app.bl
      .rollback(app.rbBcd.data, app.rbMcu.name, app.rbMcu.data)
      .then(() => "Done: the speaker starts B&W's software. Unplug it from the computer."),
  );
  if (r) {
    app.bl = null;
    show("welcome");
  }
}
// ---- software -----------------------------------------------------------------------------------------------------

let softwareDrawn = 0;
export function drawSoftware(b) {
  // the slots are the State record's: none while the module isn't up as our OS (starting, in BL2, B&W's software)
  const st = app.state,
    slot = names(app.S.Slot);
  const runs = st ? slot[st.slot] : null;
  const failed = ["A", "B", "the rescue"].filter((_, k) => (b.failed >> k) & 1);
  // a slot's stamp is "VERSION [SHA LENGTH]": the version says it
  const line = (s, stampv) =>
    `${s.toUpperCase()}: ${stampv?.split(" ")[0] || "empty"}${runs === s ? ", running" : ""}${slot[st.good] === s ? ", the good one" : ""}`;
  $("slots").textContent =
    (st ? [line("a", st.stamp_a), line("b", st.stamp_b)].join(" · ") : "not known while the module isn't up") +
    (runs === "none" ? " · the module runs its rescue" : "") +
    (b.next ? ` · the next boot tries ${slot[b.next].toUpperCase()}` : "") +
    (b.installing ? " · the rescue installs itself into a slot" : "") +
    (failed.length ? ` · failed: ${failed.join(", ")}` : "");
  const back = $("ways-back");
  back.replaceChildren();
  const drawn = ++softwareDrawn;
  const other = runs === "a" ? "b" : runs === "b" ? "a" : null;
  const usb = app.sp.link() === "usb"; // the slots and the MCU's backups are USB's (over the network: no going back)
  if (usb && other && st[`stamp_${other}`]) {
    const x = el("button", {
      className: "quiet",
      textContent: `Back to slot ${other.toUpperCase()} (${st[`stamp_${other}`].split(" ")[0]})`,
    });
    x.onclick = async () => {
      if (
        !(await ask(
          `Start slot ${other.toUpperCase()}?`,
          "The module restarts with the system in the other slot; it stays there if it comes up, else it comes back to this one. Sound stops for about half a minute.",
          "Restart",
        ))
      )
        return;
      act(() =>
        app.sp
          .call("Boot", { op: "TRY", slot: other.toUpperCase(), restart: true })
          .then(() => notice(`The module restarts on slot ${other.toUpperCase()}.`)),
      );
    };
    back.append(x);
  }
  backups()
    .then((l) => {
      if (!l.length || !usb || noDfu() || drawn !== softwareDrawn) return; // drawn again meanwhile: that one adds it
      const x = el("button", {
        className: "quiet",
        textContent: `Restore the MCU firmware ${l[0].holds} (backed up ${new Date(l[0].date).toLocaleDateString()})`,
      });
      x.onclick = () => restoreBackup(l[0]);
      back.append(x);
    })
    .catch(() => {});
}

/// The releases (images/index.json: `releases`, newest first, each with `version`, `date`, `notes` and its images
/// `mcu`, `system`, `bcd` as {file, version, size, sha256}; docs/web.md, Releases). The hosted page's come with it;
/// `zeppelin web` serves this computer's build as one release. Nothing is downloaded until it is installed.
export let releases = [];
export async function loadReleases() {
  try {
    const r = await fetch("images/index.json", { cache: "no-store" });
    if (r.ok) releases = (await r.json()).releases ?? [];
  } catch {
    releases = [];
  }
  app.drawnFor = "";
  drawInstall();
  if (app.sp && app.info) drawReleases();
  if (app.bl) drawDfu();
}

/// The welcome view's Install: the release it puts on, and the Lightning model's form: an alpha, so only when a release
/// has its file and the person ticked that they'll try one.
export function drawInstall() {
  const rel = releases[0],
    fw = releases.find((r) => r.fw);
  $("install-release").textContent = rel
    ? `It installs release ${rel.version}${rel.date ? `, ${rel.date}` : ""}.${fw ? "" : " The Lightning model's file isn't in a release yet."}`
    : "No release is offered to this page: the install needs one.";
  // B&W's file and the address go any time (the way back); our release's only with one, and the alpha ticked
  $("zd1l-alpha").disabled = !fw;
  $("zd1l-install").disabled = !fw || !$("zd1l-alpha").checked;
}

/// A release's image, fetched and checked against the manifest's SHA-256.
export async function fetchImage(img) {
  const r = await fetch(`images/${img.file}`, { cache: "no-store" });
  if (!r.ok) throw new Error(`${img.file}: ${r.status} ${r.statusText}`);
  const data = new Uint8Array(await r.arrayBuffer());
  if (img.sha256) {
    const h = [...new Uint8Array(await crypto.subtle.digest("SHA-256", data))]
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    if (h !== img.sha256) throw new Error(`${img.file}: not the file the release lists (its SHA-256 differs)`);
  }
  return data;
}

/// Where a running version stands against the releases: its position in the list (versions are commits: they don't
/// order), -1 for a build the list doesn't have.
export const position = (running, kind) => (running ? releases.findIndex((r) => r[kind]?.version === running) : -1);

/// The main board takes no firmware over USB: B&W's bootloader on a Zeppelin Air with Lightning (and an A7) has no USB
/// DFU. Its firmware comes from the module's system instead (the MCU follows its OS), once a person at the speaker
/// allows it (ON/OFF held 5 s; docs/mcu/bootloader-hbl.md), and the module's boot image from B&W's updater
/// ([`BOOT_BY_IP`]). The MCU says which model it is (`Info.model`); a firmware older than that field said so when it
/// refused DFU (`app.noDfu`: that firmware's version, connection.js).
export const noDfu = () => ["zd1l", "a7"].includes(modelOf(app.info)) || (!!app.noDfu && app.noDfu === app.info?.fw);

/// Where a Lightning model's boot image comes from: B&W's updater, as at its first install (js/zd1l.js).
const BOOT_BY_IP =
  "On the Lightning model the module's boot image goes on through B&W's updater, as at the first install: unplug " +
  "the speaker from the mains, hold ON/OFF while plugging it in (its light goes white), connect it to the router with " +
  "an Ethernet cable, then the welcome view's Lightning install by its address. The settings stay. It can wait: the " +
  "system runs on the boot image it has.";

/// The banner on top: what there is to do, with the one button (none: it's for the person at the speaker).
function banner(title, text, label, f, notes) {
  const u = label ? el("button", { className: "primary", textContent: label }) : null;
  if (u) u.onclick = f;
  $("update").replaceChildren(
    el("i", { className: "lamp on" }),
    el(
      "div",
      { className: "banner-text" },
      el("strong", { textContent: title }),
      el("span", { className: "small", textContent: text }),
      notes
        ? el(
            "details",
            {},
            el("summary", { textContent: "What's new" }),
            el("p", { className: "small notes", textContent: notes }),
          )
        : null,
    ),
    u,
  );
  $("update").hidden = false;
}

/// The release's first system that removes the MCU app it staged once the main board runs it: before it (or with
/// the main board on another app) one is staged, which a rescue from before it serves B&W's bootloader instead of
/// starting BL2 (docs/mcu/bootloader-hbl.md).
const FIRST_UNSTAGING = "0.5.1";

/// What the module runs, for the plan: our OS from a slot, its rescue, BL2 with no image, B&W's software; null while it
/// starts (judged once it is up).
function moduleNow() {
  const ms = names(app.S.ModState)[app.info?.mod_state];
  if (ms === "ready") return app.info.mod_os?.split(" ")[2] === "rescue" ? "rescue" : "os";
  return ms === "bl2" || ms === "stock" ? ms : null;
}

/// The facts the update plan starts from (host/core's plan.rs, `planUpdate`): what is behind `rel`, the model and the
/// link, what the main board's firmware can do, Ethernet, an MCU app staged on the module. Null while the module starts.
function planFacts(rel) {
  const module = moduleNow();
  if (!module) return null;
  const fw = app.info.fw,
    os = app.info.mod_os?.split(" ")[1];
  const ours = module === "os" || module === "rescue";
  const sysAt = position(os, "system"),
    firstAt = releases.findIndex((r) => r.version === FIRST_UNSTAGING);
  return {
    model: noDfu() ? "zd1l" : "zd1",
    link: app.sp.link(),
    module,
    boot: ours ? bootBehind(rel) : true,
    system: !!rel.system && (module !== "os" || position(os, "system") !== 0),
    mcu: !!rel.mcu && position(fw, "mcu") !== 0,
    rescueIsRelease: module === "rescue" && os === rel.system?.version,
    ethernet: ours && !!app.state?.eth_link && !!app.state?.eth_ip,
    allow: app.sp.supports("Allow"),
    updater: app.sp.supports("Bl2Updater"),
    staged: !(sysAt >= 0 && (firstAt < 0 || sysAt <= firstAt) && releases[sysAt].mcu?.version === fw),
    has: { system: !!rel.system, signed: !!rel.signed, bcd: !!rel.bcd, mcu: !!rel.mcu, fw: !!rel.fw },
  };
}

/// What each route of a plan does, for a person (the confirmation lists them in order).
const ROUTES = {
  "system-usb": () =>
    "the module's system goes into the slot that doesn't run (the old one stays as the way back) and starts once",
  "system-follow": () =>
    "the module's system goes into the slot that doesn't run (the old one stays as the way back); someone at the " +
    "speaker allows the main board the firmware it carries (ON/OFF held 5 s while its light flashes amber); the slot " +
    "starts once, and the main board leaves USB for about a minute, its light blinking white, and comes back with it",
  follow: () =>
    "someone at the speaker allows the main board the firmware the module's system carries (ON/OFF held 5 s while its " +
    "light flashes amber); the module restarts, and the main board leaves USB for about a minute and comes back with it",
  "system-net": (st) =>
    "someone at the speaker allows it (ON/OFF held 5 s while its light flashes amber); the module's system goes into " +
    "the slot that doesn't run (the old one stays as the way back) and the speaker restarts on it" +
    (st.mcu ? ", the main board taking its firmware from it" : "") +
    "; this page connects again by itself",
  rescue: () => "the release's system goes from the boot image into the slot that doesn't run, and starts once",
  dfu: () =>
    "the main board's firmware goes on over its bootloader (a backup of it first: a download, and a copy in this browser)",
  relay: () =>
    "the module's boot image (its start-up, and the release's system as its rescue) is rewritten through the " +
    "bootloader, about 6 minutes (keep this tab in front: a browser slows a tab in the background); the release's " +
    "system then goes from it into the slot that doesn't run",
  install: () =>
    "the module's boot image (its start-up, and the release's system as its rescue) and the main board's firmware (a " +
    "backup of it first) go on from the bootloader, about 7 minutes (keep this tab in front); the release's system " +
    "then goes from the boot image into the slot that doesn't run",
  "bl2-xmodem": () =>
    "the module's boot image goes on again over the speaker's own link, about 5 minutes (keep this tab in front); its " +
    "system and settings stay",
  updater: () =>
    "someone at the speaker starts its updater (ON/OFF held 5 s while its light flashes amber, then let go); the " +
    "speaker leaves this page, its module runs B&W's updater on the Ethernet cable, which takes the release's file for " +
    "both chips, and it restarts with our software, the release's system going into the slot that doesn't run: about " +
    "ten minutes, its power and cable in. Nothing written, a power cut takes it back to what it had",
};

/// The parts a plan leaves (`left`), and why, with what a person could do (`hand`: the plan with routes done by hand).
function leftText(facts, plan, hand) {
  const parts = [
    plan.left.boot && "the boot image",
    plan.left.system && "the system",
    plan.left.mcu && "the main board's firmware",
  ].filter(Boolean);
  if (!parts.length) return "";
  const what = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}` : parts[0];
  if (hand.steps.some((s) => s.route === "key-hold"))
    return `${what[0].toUpperCase()}${what.slice(1)}: by hand. ${BOOT_BY_IP}`;
  if (facts.link === "net")
    return `${what[0].toUpperCase()}${what.slice(1)}: over USB (connect it with a cable, then Update).`;
  return `${what[0].toUpperCase()}${what.slice(1)}: not from here with this release's files.`;
}

/// The update for `rel`: its plan (host/core's plan.rs, without routes done by hand), what it leaves and why. Null
/// while the module starts.
function planFor(rel) {
  const facts = planFacts(rel);
  if (!facts) return null;
  const plan = planUpdate(facts, false),
    hand = planUpdate(facts, true);
  return { facts, plan, left: leftText(facts, plan, hand) };
}

/// The banner on top for a speaker that isn't the release's: what is behind, the plan's one button, what it leaves.
/// The module on its rescue, in BL2 with no boot image, or on B&W's software: an install or an update that stopped
/// (the person away, the page closed), picked up from where it is.
function drawUpdate(rel, where, notes) {
  const p = planFor(rel);
  if (!p) return false;
  const { facts, plan, left } = p;
  const behind = facts.boot || facts.system || facts.mcu || facts.module !== "os";
  if (!behind) return false;
  const title =
    facts.module === "bl2"
      ? "The module's boot image is missing"
      : facts.module === "stock" || facts.module === "rescue"
        ? "The install isn't finished"
        : `Release ${rel.version} is available`;
  const lead =
    facts.module === "bl2"
      ? "The module stopped in its bootloader with no boot image to start: an update of it didn't finish."
      : facts.module === "stock"
        ? "The module still runs B&W's software."
        : facts.module === "rescue"
          ? "The module runs its rescue: it has no system in a slot yet."
          : where;
  const mins = Math.max(1, Math.round(plan.secs / 60));
  const text = [lead, plan.steps.length ? `About ${mins} minute${mins > 1 ? "s" : ""}.` : "", left]
    .filter(Boolean)
    .join(" ");
  banner(
    title,
    text,
    plan.steps.length ? (facts.module === "os" ? `Update to ${rel.version}` : `Finish: install ${rel.version}`) : null,
    () => runPlan(rel),
    facts.module === "os" ? notes : null,
  );
  return true;
}

/// Run the plan for `rel` (planned again now: the speaker may have moved on since it was drawn), each route with the
/// flows the page has, the connection back after a route that ends it.
async function runPlan(rel) {
  const p = planFor(rel);
  if (!p?.plan.steps.length) return notice(p?.left || "Nothing to do from here.", "warn");
  const { facts, plan, left } = p;
  const said = plan.steps.map((st) => ROUTES[st.route](st)).join("; then ");
  if (
    !(await ask(
      `Update to ${rel.version}?`,
      `${said[0].toUpperCase()}${said.slice(1)}. About ${Math.max(1, Math.round(plan.secs / 60))} minutes; sound stops ` +
        `meanwhile.${left ? ` ${left}` : ""}`,
      "Update",
    ))
  )
    return;
  // the Zeppelin Air's boot image through BL2: a firmware too old to say the model could be on a Lightning model's
  if (
    plan.steps.some((st) => st.route === "bl2-xmodem") &&
    modelOf(app.info) !== "zd1" &&
    !(await ask(
      "A Zeppelin Air (30-pin dock)?",
      "This writes the boot image of the Zeppelin Air with the 30-pin dock. The main board's firmware is too old to " +
        `say which model this is. On a Zeppelin Air with Lightning don't: ${BOOT_BY_IP}`,
      "It's the 30-pin one: write it",
    ))
  )
    return;
  const ctx = { addr: app.state?.eth_ip, link: facts.link, said: [], stop: null };
  const r = await job(`Update to ${rel.version}`, false, async () => {
    for (const [i, st] of plan.steps.entries()) {
      events.phase(i + 1, plan.steps.length, ROUTE_PHASE[st.route]);
      await STEPS[st.route](rel, st, ctx);
      if (ctx.stop) break;
    }
    return `Done: ${ctx.said.join(", ")}.${ctx.stop ? ` ${ctx.stop}` : left ? ` ${left}` : ""}`;
  });
  if (!app.sp) {
    if (ctx.link === "net" && ctx.addr) await connectNet(ctx.addr);
    else await connect(false);
  } else refresh(true);
  return r;
}

/// Each route's phase, as the job strip names it.
const ROUTE_PHASE = {
  "system-usb": "the module's system",
  "system-follow": "the module's system, the main board's firmware from it",
  follow: "the main board's firmware, from the module",
  "system-net": "the module's system",
  rescue: "the module's system",
  dfu: "the main board's firmware",
  relay: "the module's boot image",
  install: "the module's boot image and the main board's firmware",
  "bl2-xmodem": "the module's boot image",
  updater: "the speaker's updater",
};

/// A slot's verdict, an error if the module went back to another.
function keptSlot(r) {
  if (r?.outcome === "fellBack")
    throw new Error(
      `The system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`,
    );
}

/// A route that ended the connection (the bootloader, the updater): the speaker back for the next.
async function back(after) {
  await connect(false);
  if (!app.sp) throw new Error(`The speaker didn't come back after ${after}: connect again, then update.`);
}

/// The hold that allows the main board an app from the module (`Allow`): false if nobody held ON/OFF (the main board
/// keeps its firmware, and the plan stops after this route: what follows counts on it).
async function allowed(ctx) {
  try {
    await app.sp.allow();
    return true;
  } catch (e) {
    if (e?.code !== "DENIED") throw e;
    ctx.stop = `The main board kept ${app.info?.fw ?? "its firmware"} (ON/OFF wasn't held): Update again for the rest.`;
    return false;
  }
}

/// The module up as our OS again (after the main board took its app: at that ready the system removes the copy it
/// staged, which an old rescue would serve B&W's bootloader), 2 minutes at most.
async function moduleUp() {
  const t0 = Date.now();
  while (Date.now() - t0 < 120_000) {
    const i = await app.sp.call("Info", {});
    if (names(app.S.ModState)[i.mod_state] === "ready") return new Promise((r) => setTimeout(r, 5000));
    events.wait("the module starting", Math.round((Date.now() - t0) / 1000));
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error("The module didn't come up within 2 minutes: Diagnostics says where it is.");
}

/// The routes (host/core's plan.rs `Route`), with the flows the page has.
const STEPS = {
  async "system-usb"(rel, _st, ctx) {
    const r = await app.sp.updateSystem(await fetchImage(rel.system), true);
    keptSlot(r);
    ctx.said.push(`system ${rel.system.version} in slot ${r.slot.toUpperCase()}`);
  },

  /// The system over USB, the hold after it is written and before its slot starts (the main board takes the app at
  /// that start's ready: mcu/src/mcufw.c, module/os/apps/zd1-linkd), then the main board back with it.
  async "system-follow"(rel, _st, ctx) {
    const w = await app.sp.updateSystem(await fetchImage(rel.system), false);
    const ok = await allowed(ctx);
    let r = null;
    try {
      r = await app.sp.startSlot(w.slot);
    } catch (e) {
      // allowed, the main board leaves USB at the slot's ready (the module asks it then), maybe before the verdict
      // was read: it comes once the main board is back
      if (!ok || !GONE.includes(e?.code)) throw e;
      app.sp.close();
      app.sp = null;
    }
    keptSlot(r);
    if (ok) {
      const i = await mcuFromModule(rel.mcu.version, !!r);
      if (!r) {
        // the good slot, which the main board keeps (its EEPROM) through its bootloader: the one tried if it got ready
        const b = await app.sp.call("BootState", {});
        const good = names(app.S.Slot)[b.good];
        if (good !== w.slot)
          throw new Error(
            `The main board runs ${i.fw}, but the system in slot ${w.slot.toUpperCase()} didn't come up: the module ` +
              `is back on slot ${good.toUpperCase()}.`,
          );
      }
      ctx.said.push(`system ${rel.system.version} in slot ${w.slot.toUpperCase()}`, `main board ${i.fw}`);
      await moduleUp();
    } else ctx.said.push(`system ${rel.system.version} in slot ${(r?.slot ?? w.slot).toUpperCase()}`);
  },

  /// The hold, then a ready for the module to ask at (it asks at each one; the main board takes it only allowed).
  async follow(rel, _st, ctx) {
    if (!(await allowed(ctx))) return;
    await app.sp.call("Module", { op: "RESTART", timeout_s: 0 });
    const i = await mcuFromModule(rel.mcu.version);
    ctx.said.push(`main board ${i.fw}`);
    await moduleUp();
  },

  /// The signed system over the network: the speaker checks its signature and asks for the hold itself, which allows
  /// the main board the firmware it brings too (docs/protocol/net.md).
  async "system-net"(rel, _st, ctx) {
    const r = await app.sp.updateSystem(await fetchImage(rel.signed), true);
    if (r.outcome === "fellBack")
      throw new Error(
        `The system in slot ${r.slot.toUpperCase()} didn't come up: the speaker is back on slot ${r.now.toUpperCase()}.`,
      );
    ctx.said.push(`system ${rel.signed.version} in slot ${r.slot.toUpperCase()}`);
  },

  async rescue(rel, _st, ctx) {
    const r = await app.sp.installFromRescue();
    keptSlot(r);
    ctx.said.push(`system ${rel.system.version} in slot ${r.slot.toUpperCase()}`);
  },

  async dfu(rel, _st, ctx) {
    const r = await app.sp.flashMcu(rel.mcu.file, await fetchImage(rel.mcu));
    app.sp = null;
    ctx.said.push(r.matches ? `main board ${r.fw}` : `main board: ${r.fw} runs, not the one written`);
    await back("its main board's firmware");
  },

  async relay(rel, _st, ctx) {
    const bcd = await fetchImage(rel.bcd);
    const sp = app.sp;
    app.sp = null; // through the bootloader: this connection ends
    const r = await sp.updateBoot(bcd);
    if (r.outcome === "fellBack")
      throw new Error(
        `The boot image went on, but its system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`,
      );
    ctx.said.push(`boot image ${rel.bcd.version}, system ${r.os.split(" ")[1]} in slot ${r.now.toUpperCase()}`);
    await back("its boot image");
  },

  async install(rel, _st, ctx) {
    const [bcd, mcu] = [await fetchImage(rel.bcd), await fetchImage(rel.mcu)];
    const sp = app.sp;
    app.sp = null; // through the bootloader: this connection ends
    const r = await sp.install(bcd, rel.mcu.file, mcu, undefined); // the boot image's rescue is the system
    ctx.said.push(`boot image ${rel.bcd.version}, main board ${r.fw}, system ${r.os} in slot ${r.slot.toUpperCase()}`);
    await back("the install");
  },

  async "bl2-xmodem"(rel, _st, ctx) {
    const r = await app.sp.updateBootInBl2(await fetchImage(rel.bcd));
    ctx.said.push(`boot image ${rel.bcd.version}, the module runs ${r.os}`);
  },

  /// The Lightning model's updater (`Bl2Updater`): the main board into B&W's bootloader, the module into BL2's web
  /// updater on its Ethernet, looked for at the speaker's Ethernet address (BL2 asks DHCP for itself), the release's .fw
  /// to it (zd1l.js `writeFw`), until our software answers again.
  async updater(rel, _st, ctx) {
    const fw = await fetchImage(rel.fw);
    const sp = app.sp;
    app.sp = null; // it leaves: this connection ends
    try {
      await sp.bl2Updater();
    } catch (e) {
      app.sp = sp; // nobody held ON/OFF (or it refused): the speaker stays as it was
      throw e;
    }
    await updaterAt(ctx.addr);
    const r = await writeFw(ctx.addr, fw, rel.fw.file.split("/").pop(), { own: true, phase: 1, of: 2 });
    ctx.said.push(r.replace(/\.$/, ""));
  },
};

/// What the releases are drawn for: the versions running, and the boot image's once the module's State record is in.
export const drawnKey = () => `${app.info?.fw}|${app.info?.mod_os}|${app.info?.mod_state}|${moduleBoot()}`;

/// The boot image the module started from (the State record's `boot`: "" from before it had one), or undefined while
/// that isn't known: the module not up as our OS, or the record from before it last started (a new `os`).
function moduleBoot() {
  const st = app.state;
  if (names(app.S.ModState)[app.info?.mod_state] !== "ready" || !st || st.os !== app.info.mod_os) return undefined;
  return st.boot ?? "";
}

/// The release's boot image isn't the one the module started from (by its stub and kernel: `zd1.boot=`); false while
/// that isn't known (the module starting, no State record).
function bootBehind(rel) {
  const boot = moduleBoot();
  return !!rel.bcd?.version && boot !== undefined && boot !== rel.bcd.version;
}

export function drawReleases() {
  const fw = app.info?.fw,
    os = app.info?.mod_os?.split(" ")[1];
  app.drawnFor = drawnKey();
  const v = $("verdict");
  v.replaceChildren();
  $("update").hidden = true;
  const rel = releases[0];
  if (!rel) {
    v.append(
      el("p", { className: "small", textContent: "No release is offered to this page (installs from a file below)." }),
    );
  } else {
    const [pm, ps] = [position(fw, "mcu"), position(os, "system")];
    const module = moduleNow();
    // what is behind, each with where it stands: "the main board's firmware (2 releases behind)"
    const part = (p, what, running) =>
      p === 0
        ? null
        : `${what} (${p > 0 ? `${p} release${p > 1 ? "s" : ""} behind` : running ? `${running}, not a release` : "not ours"})`;
    const and = (l) => (l.length < 2 ? (l[0] ?? "") : `${l.slice(0, -1).join(", ")} and ${l.at(-1)}`);
    // the boot image: rewritten only when the release's stub or kernel differ from the ones the module started from
    // (bootBehind; module/os/assemble.py, payload)
    const boot = moduleBoot();
    const behind = bootBehind(rel);
    const updates = and(
      [
        rel.mcu && part(pm, "the main board's firmware", fw),
        rel.system && module === "os" && part(ps, "the module's system", os),
        behind && `the module's boot image${boot ? "" : " (from before boot images had a version)"}`,
      ].filter(Boolean),
    );
    const named = `release ${rel.version}${rel.date ? ` (${rel.date})` : ""}`;
    const where = updates ? `It updates ${updates}.` : "";
    // behind (or an install that stopped): the banner on top says so, with the plan's one button (host/core plan.rs)
    const shown = drawUpdate(rel, where, rel.notes);
    v.append(
      el("p", {
        textContent: !module
          ? `Latest: ${named}. The module is starting: what it runs is checked once it is up.`
          : module !== "os"
            ? "The install stopped on the way: the banner on top finishes it."
            : shown
              ? `Latest: ${named}. ${where}`
              : `Up to date: ${named}.`,
      }),
    );
    if (rel.notes) v.append(el("p", { className: "small notes", textContent: rel.notes }));
  }
  const rows = releases.slice(1).map((r) => {
    const row = el(
      "div",
      { className: "row item" },
      el("span", { textContent: `${r.version}${r.date ? `, ${r.date}` : ""}` }),
      el("span", { className: "grow" }),
    );
    if (r.notes) row.title = r.notes;
    for (const [kind, label] of [
      ["system", "its system"],
      ["mcu", "its MCU firmware"],
    ]) {
      if (!r[kind] || (kind === "mcu" && (app.sp?.link() === "net" || noDfu()))) continue; // the MCU's: USB DFU only
      const b = el("button", { className: "quiet", textContent: `Install ${label}` });
      b.onclick = () => installRelease(r, kind);
      row.append(b);
    }
    return row;
  });
  const d = el(
    "details",
    {},
    el("summary", { className: "small", textContent: `${rows.length} earlier release${rows.length === 1 ? "" : "s"}` }),
    ...rows,
  );
  $("builds").replaceChildren(...(rows.length ? [d] : []));
  $("builds").hidden = !rows.length;
}

/// The updater answering at `addr` after `Bl2Updater` (about a minute: the module starts again under B&W's bootloader,
/// then reboots into BL2). Where its answers can be read (zd1-app), by what it says; in a browser blind: something
/// answers there that isn't our software, twice 5 s apart.
async function updaterAt(addr) {
  const t0 = Date.now();
  let seen = 0;
  while (Date.now() - t0 < 240_000) {
    events.wait(`the speaker's updater coming up at ${addr}`, Math.round((Date.now() - t0) / 1000));
    await new Promise((r) => setTimeout(r, 5000));
    let kind = null;
    try {
      kind = (await bl2Look(addr, { signal: AbortSignal.timeout(4000) })).kind;
    } catch {}
    if (kind === "zd1l") return;
    seen = kind === "blind" && !(await ours(addr)) ? seen + 1 : 0;
    if (seen >= 2) return;
  }
  throw new Error(
    `The speaker's updater didn't answer at ${addr} within 4 minutes. If the speaker's light is white, it runs ` +
      "under another address: the welcome view's Lightning install finds it (or takes it from the router's list) and " +
      "puts the release on. Else unplug the speaker's power and plug it in again: it starts what it had.",
  );
}

/// The errors of a USB device going away mid-call: gone, or a transfer that failed or timed out with it.
const GONE = ["GONE", "DEVICE", "TIMEOUT"];

/// The main board taking the app the module's system carries, allowed (`STEPS` system-follow, follow): it leaves USB for its
/// bootloader, which powers the module and takes the app over SPI (about 45 s, its light blinking white), and comes back
/// running `version` (`connected`: still there, else it has left already). Its Info once it does; an error if it doesn't
/// leave within 3 minutes, or isn't back with it in 5.
async function mcuFromModule(version, connected = true) {
  const t0 = Date.now(),
    secs = () => Math.round((Date.now() - t0) / 1000);
  const pause = (ms) => new Promise((r) => setTimeout(r, ms));
  let left = !connected;
  while (Date.now() - t0 < 300_000) {
    events.wait(
      left ? "the main board takes its firmware from the module" : "the module asks the main board to take it",
      secs(),
    );
    await pause(2000);
    if (!app.sp) {
      // back on USB once its bootloader started the app (the WebUSB grant holds for it: a new USB serial may need
      // Connect again)
      app.sp = (await Speaker.connect(events).catch(() => null)) ?? null;
      continue;
    }
    let i;
    try {
      i = await app.sp.call("Info", {});
    } catch (e) {
      if (!GONE.includes(e?.code)) throw e;
      app.sp.close();
      app.sp = null;
      left = true;
      continue;
    }
    app.info = i;
    if (i.fw === version) {
      events.step(`the main board runs ${i.fw} (${secs()} s)`);
      return i;
    }
    if (!left && Date.now() - t0 > 180_000)
      throw new Error(
        `The main board still runs ${i.fw}: the module didn't hand it ${version}. Diagnostics' log says why (its ` +
          "[zd1-linkd] and [mcufw] lines).",
      );
  }
  throw new Error(
    `The main board didn't come back on USB with ${version} in 5 minutes. If its light is steady again, connect again ` +
      "(the browser may ask for it once more); if it stays white, B&W's updater has the way back (the Lightning " +
      "install by its address, after ON/OFF held while plugging it in).",
  );
}

export async function installRelease(rel, kind) {
  if (kind === "system" && app.sp.link() === "net") {
    if (!rel.signed)
      return notice(`Release ${rel.version} has no image signed for the network: install it over USB.`, "warn");
    kind = "signed";
  }
  let data;
  try {
    data = await fetchImage(rel[kind]);
  } catch (e) {
    return failed(e);
  }
  return kind === "mcu" ? installMcu(rel[kind].file, data) : installSystem(rel[kind].file, data);
}

export async function installSystem(name, data) {
  let i;
  try {
    i = imageInfo(name, data);
  } catch (e) {
    return failed(e);
  }
  const net = app.sp.link() === "net";
  if (net && i.kind !== "signed")
    return notice(`${name}: over the network a system must be signed (a release's .signed image).`, "warn");
  if (i.kind !== "system" && i.kind !== "signed")
    return notice(`${name} is not a system image (a gzip'd cpio).`, "warn");
  if (
    !(await ask(
      "Install the module's system?",
      `${name}: version ${i.version}, ${(i.size / 1048576).toFixed(1)} MB. It goes ` +
        "into the slot that doesn't run, then the module starts it once: kept if it comes up, else the old one comes back. " +
        "About a minute and a half; sound stops while the module restarts.",
      "Install",
    ))
  )
    return;
  await job(`System ${i.version}`, true, async () => {
    const r = await app.sp.updateSystem(data, true);
    return r.outcome === "running"
      ? `Done: slot ${r.slot.toUpperCase()} runs ${i.version}.`
      : r.outcome === "installed"
        ? `Installed in slot ${r.slot.toUpperCase()}.`
        : `Slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`;
  });
  refresh(true);
}

/// MCU firmware from the running firmware or the bootloader: a .hex, the app region's .bin, or a backup (a download,
/// or one this browser kept).
export async function installMcu(name, data) {
  let i;
  try {
    i = imageInfo(name, data);
  } catch (e) {
    return failed(e);
  }
  if (i.kind !== "mcu" && i.kind !== "backup")
    return notice(`${name} is not MCU firmware (Intel HEX, the app region's .bin, or a backup).`, "warn");
  if (app.sp?.link() === "net")
    return notice("The main board's firmware goes over USB (over the network it follows its system).", "warn");
  if (app.sp && noDfu())
    return notice(
      "This speaker's main board takes no firmware over USB (B&W's bootloader on the Lightning model has none): it " +
        "comes from the module's system, which the update banner asks for.",
      "warn",
    );
  const from = i.kind === "backup" ? ` (the backup read ${new Date(i.when).toLocaleString()})` : "";
  if (
    !(await ask(
      "Install the MCU firmware?",
      `${name}${from}: ${i.version ?? "no version in it (B&W's firmware, or a backup of it)"}${i.built ? `, built ${i.built}` : ""}. ` +
        (app.bl ? "" : "The speaker restarts into its bootloader; ") +
        "this page saves a backup of the firmware on it (a download, and a copy in this browser) before anything is written, " +
        "then writes, checks and starts the new one. About 20 s; the browser may ask once for the bootloader or the firmware.",
      "Install",
    ))
  )
    return;
  const target = app.sp ?? app.bl;
  const r = await job(`MCU firmware ${i.version ?? ""}`, false, () =>
    target
      .flashMcu(name, data)
      .then((r) =>
        r.matches
          ? `Done: ${r.fw} runs.`
          : r.fw
            ? `It runs ${r.fw}, not the version just written.`
            : "Written and started.",
      ),
  );
  // done or not, this connection has ended (the flash took the speaker into its bootloader): whatever is on USB
  // now, the firmware or the bootloader, the page shows it
  app.sp = null;
  app.bl = null;
  await connect(false);
}
