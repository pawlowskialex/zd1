// The software: the bootloader's view (install, repair, roll back), the slots, the releases, installs from files.
import { imageInfo, Speaker } from "../pkg/zd1_web.js";
import { backups, drawBackups, restoreBackup } from "./backups.js";
import { act, connect, events, job, show } from "./connection.js";
import { app } from "./shared.js";
import { modelOf, refresh } from "./speaker.js";
import { $, ask, el, failed, names, notice } from "./ui.js";

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

/// An install that stopped on the way (the person away, the page closed): the state it left, picked up from there.
/// Our firmware with the module on the stock software (the boot image never went on): the whole install again. The
/// module on its rescue, no system in a slot: the release's rescue installs itself (the MCU first if it is behind:
/// Boot INSTALL is the release's); an older rescue gets the release's whole update when its boot image differs (the
/// boot image and the MCU, then the new rescue installs itself: never the release's system on an older boot image and
/// MCU), else the system into a slot. The module in BL2: an update of its boot image stopped after BL2 erased the old
/// one: the boot image through BL2's XModem.
function unfinished(rel) {
  const ms = names(app.S.ModState)[app.info.mod_state];
  // a Lightning model: the release's boot image is the Zeppelin Air's (its HWID, its CnE), never for it
  if (ms === "bl2" && noDfu()) {
    banner(
      "The module's boot image is missing",
      `The module stopped in its bootloader with no boot image to start: an update of it didn't finish. ${BOOT_BY_IP}`,
    );
    return true;
  }
  if (ms === "stock" && noDfu()) {
    banner(
      "The install isn't finished",
      "The module still runs B&W's software. On the Lightning model ours goes on through B&W's updater: unplug the " +
        "speaker from the mains, hold ON/OFF while plugging it in (its light goes white), connect it to the router with " +
        "an Ethernet cable, then the welcome view's Lightning install by its address.",
    );
    return true;
  }
  if (ms === "bl2" && rel.bcd?.ours) {
    banner(
      "The module's boot image is missing",
      "The module stopped in its bootloader with no boot image to start: an update of it didn't finish. It goes on " +
        "again over the speaker's own link (about 5 minutes; keep this tab in front). Its system and settings stay.",
      `Write ${rel.version}'s boot image`,
      () => writeBootInBl2(rel),
    );
    return true;
  }
  if (ms === "stock" && rel.mcu && rel.system && rel.bcd) {
    banner(
      "The install isn't finished",
      "The module still runs B&W's software: its boot image, then the system go on (about 5 minutes).",
      `Finish: install ${rel.version}`,
      () => finishAll(rel),
    );
    return true;
  }
  const os = app.info.mod_os?.split(" ") ?? [];
  if (ms === "ready" && os[2] === "rescue" && rel.system) {
    const mcu = rel.mcu && position(app.info.fw, "mcu") !== 0 ? ["mcu"] : [];
    const needs =
      os[1] === rel.system.version
        ? [...mcu, "rescue"]
        : bootBehind(rel)
          ? ["boot", ...mcu, "system"]
          : ["system", ...mcu];
    banner(
      "The install isn't finished",
      "The module runs its rescue: it has no system in a slot yet. " +
        (needs.includes("boot") && noDfu()
          ? "The system goes on (about 2 minutes); the boot image through B&W's updater later (Software says how)."
          : needs.includes("boot")
            ? `${rel.version}'s boot image goes on, then its system from it (about 8 minutes).`
            : needs.includes("rescue")
              ? "It installs itself into one (about a minute)."
              : "The system goes on (about 2 minutes)."),
      `Finish: install ${rel.version}`,
      () => updateTo(rel, needs, true),
    );
    return true;
  }
  return false;
}

async function writeBootInBl2(rel) {
  // the Zeppelin Air's boot image: a firmware too old to say the model could be on a Lightning model's main board
  if (
    modelOf(app.info) !== "zd1" &&
    !(await ask(
      "A Zeppelin Air (30-pin dock)?",
      "This writes the boot image of the Zeppelin Air with the 30-pin dock. The main board's firmware is too old to " +
        `say which model this is. On a Zeppelin Air with Lightning don't: ${BOOT_BY_IP}`,
      "It's the 30-pin one: write it",
    ))
  )
    return;
  await job(`Write the module's boot image (${rel.version})`, false, async () => {
    const r = await app.sp.updateBootInBl2(await fetchImage(rel.bcd));
    return `Done: the module runs ${r.os}.`;
  });
  refresh(true);
}

async function finishAll(rel) {
  const r = await job(`Install release ${rel.version}`, false, async () => {
    const [mcu, bcd] = [await fetchImage(rel.mcu), await fetchImage(rel.bcd)];
    const sp = app.sp;
    app.sp = null; // the install takes the speaker into its bootloader: this connection ends
    const r = await sp.install(bcd, rel.mcu.file, mcu, undefined); // the boot image's rescue is the system
    return `Done: MCU firmware ${r.fw}, system ${r.os} in slot ${r.slot.toUpperCase()}.`;
  });
  await connect(false);
  if (r && app.sp) document.querySelector("#known").scrollIntoView({ behavior: "smooth" });
}

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
  if (rel && unfinished(rel)) {
    v.append(el("p", { textContent: "The install stopped on the way: the banner on top finishes it." }));
  } else if (!rel) {
    v.append(
      el("p", { className: "small", textContent: "No release is offered to this page (installs from a file below)." }),
    );
  } else {
    const [pm, ps] = [position(fw, "mcu"), position(os, "system")];
    // the module starting (after an update, a power-up): not judged until it is up
    const starting = ["off", "reset", "booting", "up"].includes(names(app.S.ModState)[app.info?.mod_state]);
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
    const needs = [
      rel.system && !starting && ps !== 0 && "system",
      behind && "boot",
      rel.mcu && pm !== 0 && "mcu",
    ].filter(Boolean);
    const updates = and(
      [
        rel.mcu && part(pm, "the main board's firmware", fw),
        rel.system && !starting && part(ps, "the module's system", os),
        behind && `the module's boot image${boot ? "" : " (from before boot images had a version)"}`,
      ].filter(Boolean),
    );
    const where = `It updates ${updates}.${starting ? " The module is starting: its system is checked once it is up." : ""}`;
    const named = `release ${rel.version}${rel.date ? ` (${rel.date})` : ""}`;
    v.append(el("p", { textContent: needs.length ? `Latest: ${named}. ${where}` : `Up to date: ${named}.` }));
    // a Lightning model: its boot image isn't the page's to write (BOOT_BY_IP); the rest is
    const byIp = noDfu() && behind;
    if (byIp) v.append(el("p", { className: "small", textContent: BOOT_BY_IP }));
    if (rel.notes) v.append(el("p", { className: "small notes", textContent: rel.notes }));
    // behind: the banner on top of the page says so, with the one button
    const doable = byIp ? needs.filter((k) => k !== "boot") : needs;
    if (needs.length)
      banner(
        `Release ${rel.version} is available`,
        where + (byIp ? " The boot image goes on through B&W's updater (Software says how)." : ""),
        doable.length ? `Update to ${rel.version}` : null,
        () => updateTo(rel, needs),
        rel.notes,
      );
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

/// The speaker to a release, each part only if it isn't there already. With a boot image to write: the boot image
/// and the MCU firmware in one visit to the bootloader (host/core's `install`), else the boot image alone; either
/// way the release's system comes from it (the MCU has a boot image it hasn't seen install its rescue into a slot: no
/// second upload); without one: the system into a slot, then the MCU firmware.
export async function updateTo(rel, needs, finishing = false) {
  if (app.sp.link() === "net") return updateOverNetwork(rel, needs);
  if (noDfu()) return updateFromModule(rel, needs, finishing);
  const has = (k) => needs.includes(k);
  const steps = !has("boot")
    ? needs
    : has("mcu")
      ? ["boot+mcu"] // `install`: both from the bootloader, then the system from the rescue
      : ["boot"]; // the system from its rescue as well
  const parts = {
    system: "the module's system goes into the slot that doesn't run (the old one stays as the way back)",
    boot: "the module's boot image (its start-up, and the release's system as its rescue) is rewritten through the bootloader, about 6 minutes (keep this tab in front: a browser slows a tab in the background); then the release's system goes from it into the slot that doesn't run (the old one stays as the way back)",
    mcu: "the main board's firmware (a backup of it first: a download, and a copy in this browser)",
    "boot+mcu":
      "the module's boot image (its start-up, and the release's system as its rescue) and the main board's firmware (a backup of it first: a download, and a copy in this browser) go on from the bootloader, about 7 minutes (keep this tab in front: a browser slows a tab in the background); then the release's system goes from the boot image into the slot that doesn't run (the old one stays as the way back)",
    rescue:
      "the release's system goes from the boot image into the slot that doesn't run (the old one stays as the way back)",
  };
  const plan = steps.map((k) => parts[k]).join("; then ");
  if (
    !finishing &&
    !(await ask(
      `Update to ${rel.version}?`,
      `${plan[0].toUpperCase()}${plan.slice(1)}. A few minutes${steps.includes("boot") ? " more" : ""}; sound stops meanwhile.`,
      "Update",
    ))
  )
    return;
  // a step that ends the connection (the bootloader): the next one needs it back
  const again = async (after) => {
    await connect(false);
    if (!app.sp) throw new Error(`The speaker didn't come back after ${after}: connect again, then update.`);
  };
  const r = await job(`Update to ${rel.version}`, false, async () => {
    const said = [];
    let n = 0;
    for (const [i, k] of steps.entries()) {
      const more = i < steps.length - 1;
      if (k === "system" || k === "rescue") {
        events.phase(++n, steps.length, "the module's system");
        const r =
          k === "system"
            ? await app.sp.updateSystem(await fetchImage(rel.system), true)
            : await app.sp.installFromRescue();
        if (r.outcome === "fellBack")
          throw new Error(
            `The system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`,
          );
        said.push(`system ${rel.system.version} in slot ${r.slot.toUpperCase()}`);
      } else if (k === "boot+mcu") {
        const [bcd, mcu] = [await fetchImage(rel.bcd), await fetchImage(rel.mcu)];
        const sp = app.sp;
        app.sp = null; // through the bootloader: this connection ends
        const r = await sp.install(bcd, rel.mcu.file, mcu, undefined);
        said.push(
          `boot image ${rel.bcd.version}, MCU firmware ${r.fw}, system ${r.os} in slot ${r.slot.toUpperCase()}`,
        );
      } else if (k === "boot") {
        events.phase(++n, steps.length, "the module's boot image");
        const bcd = await fetchImage(rel.bcd);
        const sp = app.sp;
        app.sp = null; // through the bootloader: this connection ends
        const r = await sp.updateBoot(bcd);
        if (r.outcome === "fellBack")
          throw new Error(
            `The boot image went on, but its system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`,
          );
        said.push(`boot image ${rel.bcd.version}, system ${r.os.split(" ")[1]} in slot ${r.now.toUpperCase()}`);
        if (more) await again("its boot image");
      } else if (k === "mcu") {
        events.phase(++n, steps.length, "the main board's firmware");
        const r = await app.sp.flashMcu(rel.mcu.file, await fetchImage(rel.mcu));
        app.sp = null;
        said.push(r.matches ? `MCU firmware ${r.fw}` : `MCU firmware: ${r.fw} runs, not the one written`);
        if (more) await again("its main board's firmware");
      }
    }
    return `Done: ${said.join(", ")}.`;
  });
  if (!app.sp) await connect(false);
  else refresh(true);
  return r;
}

/// A Lightning model over USB (`noDfu`): the module's system into the slot that doesn't run, and the main board's
/// firmware from it: a person at the speaker allows that (`Allow`: ON/OFF held 5 s) after the system is written and
/// before its slot starts; at the start's ready the module asks the MCU, which goes into B&W's bootloader and takes the
/// app over SPI (mcu/src/mcufw.c, module/os/apps/zd1-linkd). With the system already there the module restarts for that
/// ready. Its boot image isn't for USB here (`BOOT_BY_IP`).
async function updateFromModule(rel, needs, finishing) {
  const has = (k) => needs.includes(k);
  const sys = has("system") ? "system" : has("rescue") ? "rescue" : null;
  const mcu = has("mcu") ? rel.mcu?.version : null;
  if (!sys && !mcu) return notice(BOOT_BY_IP, "warn");
  const plan = [
    sys === "system" &&
      "the module's system goes into the slot that doesn't run (the old one stays as the way back) and starts once",
    sys === "rescue" && "the release's system goes from the boot image into the slot that doesn't run, and starts once",
    mcu &&
      `${sys ? "then " : ""}the main board takes its firmware from the module's system: someone at the speaker allows ` +
        "it (ON/OFF held 5 s while its light flashes amber), " +
        (sys ? "" : "the module restarts, ") +
        "then the main board leaves USB for about a minute, its light blinking white, and comes back with it",
  ]
    .filter(Boolean)
    .join("; ");
  if (
    !finishing &&
    !(await ask(
      `Update to ${rel.version}?`,
      `${plan[0].toUpperCase()}${plan.slice(1)}. A few minutes; sound stops meanwhile.` +
        (has("boot") ? " The boot image goes on through B&W's updater, later (Software says how)." : ""),
      "Update",
    ))
  )
    return;
  const of = (sys ? 1 : 0) + (mcu ? 1 : 0);
  const r = await job(`Update to ${rel.version}`, false, async () => {
    const said = [];
    let n = 0,
      allowed = false;
    // the hold, asked where it counts: nobody at the speaker leaves the main board as it is, the rest goes on
    const allow = async () => {
      try {
        await app.sp.allow();
        allowed = true;
      } catch (e) {
        if (e?.code !== "DENIED") throw e;
        said.push(`the main board kept ${app.info?.fw ?? "its firmware"} (ON/OFF wasn't held: Update again for it)`);
      }
    };
    let tried = null; // a slot whose verdict comes once the main board is back
    if (sys === "system") {
      events.phase(++n, of, "the module's system");
      const w = await app.sp.updateSystem(await fetchImage(rel.system), false);
      if (mcu) await allow();
      let r;
      try {
        r = await app.sp.startSlot(w.slot);
      } catch (e) {
        // allowed, the main board leaves USB at the slot's ready (the module asks it then), maybe before the verdict
        // was read: it comes after the main board is back
        if (!allowed || !GONE.includes(e?.code)) throw e;
        app.sp.close();
        app.sp = null;
        tried = w.slot;
      }
      if (r?.outcome === "fellBack")
        throw new Error(
          `The system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}` +
            (allowed ? "; the main board keeps its firmware." : "."),
        );
      if (r) said.push(`system ${rel.system.version} in slot ${r.slot.toUpperCase()}`);
    } else if (sys === "rescue") {
      events.phase(++n, of, "the module's system");
      const r = await app.sp.installFromRescue();
      if (r.outcome === "fellBack")
        throw new Error(
          `The system in slot ${r.slot.toUpperCase()} didn't come up: the module is back on slot ${r.now.toUpperCase()}.`,
        );
      said.push(`system ${rel.system.version} in slot ${r.slot.toUpperCase()}`);
    }
    if (mcu && sys !== "system") {
      await allow();
      // a ready for the module to ask at (it asks at each one, the MCU takes it only allowed)
      if (allowed) await app.sp.call("Module", { op: "RESTART", timeout_s: 0 });
    }
    if (allowed) {
      events.phase(++n, of, "the main board's firmware, from the module");
      const i = await mcuFromModule(mcu, !tried);
      said.push(`main board ${i.fw}`);
    }
    if (tried) {
      // the good slot, which the main board keeps (its EEPROM) through its bootloader: the one tried if it got ready
      const b = await app.sp.call("BootState", {});
      const good = names(app.S.Slot)[b.good];
      if (good !== tried)
        throw new Error(
          `The main board runs ${app.info.fw}, but the system in slot ${tried.toUpperCase()} didn't come up: the ` +
            `module is back on slot ${good.toUpperCase()}.`,
        );
      said.unshift(`system ${rel.system.version} in slot ${tried.toUpperCase()}`);
    }
    return `Done: ${said.join(", ")}.${has("boot") ? " The boot image: through B&W's updater (Software says how)." : ""}`;
  });
  refresh(true);
  return r;
}

/// The errors of a USB device going away mid-call: gone, or a transfer that failed or timed out with it.
const GONE = ["GONE", "DEVICE", "TIMEOUT"];

/// The main board taking the app the module's system carries, allowed (`updateFromModule`): it leaves USB for its
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

/// Over the network: the module's system alone, the release's signed image (the speaker checks its signature), allowed
/// by a person at the speaker (ON/OFF held 5 s), which allows the main board's firmware it brings too; a boot image
/// that differs needs USB (docs/protocol/net.md).
async function updateOverNetwork(rel, needs) {
  if (!needs.includes("system"))
    return notice(
      needs.includes("boot")
        ? noDfu()
          ? `This release's boot image differs from the speaker's. ${BOOT_BY_IP}`
          : "This release's boot image differs from the speaker's: that goes over USB (connect it with a cable)."
        : noDfu()
          ? "The main board's firmware doesn't match its system: it takes it from the module once someone at the " +
            "speaker allows it, which goes over USB (connect it with a cable, then Update)."
          : "The main board's firmware doesn't match its system: update over USB.",
      "warn",
    );
  if (!rel.signed)
    return notice(`Release ${rel.version} has no image signed for the network: update over USB.`, "warn");
  if (
    !(await ask(
      `Update to ${rel.version}?`,
      "Someone at the speaker allows it: ON/OFF held 5 s while its LED flashes amber. Then the module's system goes " +
        "into the slot that doesn't run (the old one stays as the way back) and the speaker restarts on it" +
        (needs.includes("mcu") ? ", the main board taking its firmware from it" : "") +
        ". A few minutes; sound stops meanwhile, and this page connects again by itself.",
      "Update",
    ))
  )
    return;
  const r = await job(`Update to ${rel.version}`, false, async () => {
    const r = await app.sp.updateSystem(await fetchImage(rel.signed), true);
    if (r.outcome === "fellBack")
      throw new Error(
        `The system in slot ${r.slot.toUpperCase()} didn't come up: the speaker is back on slot ${r.now.toUpperCase()}.`,
      );
    const boot = !needs.includes("boot")
      ? ""
      : noDfu()
        ? "; the boot image through B&W's updater (Software says how)"
        : "; the boot image goes over USB";
    return `Done: system ${rel.signed.version} in slot ${r.slot.toUpperCase()}${boot}.`;
  });
  refresh(true);
  return r;
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
