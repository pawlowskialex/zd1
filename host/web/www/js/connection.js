// The connection and the jobs: what the flows tell the page (`events`), connecting to the speaker or its
// bootloader, one operation at a time, the job strip for long flows.
import { Bootloader, newIdentity, Speaker } from "../pkg/zd1_web.js";
import { keepBackup } from "./backups.js";
import { consoleText } from "./console.js";
import { hero, heroProgress } from "./hero.js";
import { app } from "./shared.js";
import { drawDfu } from "./software.js";
import { refresh } from "./speaker.js";
import { $, ask, failed, hex4, notice } from "./ui.js";

// ---- what the flows tell the page -------------------------------------------------------------------------------

export let jobRunning = false;
/// The press arrived (or the flow went on without one): its dialog goes, whatever comes next says so.
function pressDone() {
  if ($("press").open) $("press").close();
}

export const events = {
  text(s) {
    appendLog(s);
  },
  prompt() {
    $("press-what").textContent =
      app.sp?.link() === "net"
        ? "Its light flashes purple: a press there pairs this browser with the speaker, for good (Advanced lists the " +
          "browsers and computers paired). The press does nothing else."
        : "Its light flashes purple: a press there allows this change (and the next ones for 10 minutes). The press " +
          "does nothing else.";
    if (!$("press").open) $("press").showModal(); // asked again while it waits (a person away: no deadline)
  },
  step(s) {
    pressDone();
    if (!jobRunning) return;
    $("job-log").textContent += `${s}\n`;
    $("job-now").textContent = s;
  },
  phase(n, of, what) {
    pressDone();
    if (!jobRunning) return;
    $("job-log").textContent += `\n[${n}/${of}] ${what}\n`;
    $("job-phase").textContent = `${n} of ${of}: ${what}`;
  },
  progress(_what, done, total) {
    pressDone();
    $("job-bar").value = total ? done / total : 0;
    if (jobRunning) heroProgress($("job-bar").value);
  },
  /// a wait with nothing to measure: the bar runs without a value (the browser animates it), the line says how long
  wait(what, secs) {
    pressDone();
    if (!jobRunning) return;
    $("job-bar").removeAttribute("value");
    $("job-now").textContent = `${what}: ${secs} s`;
  },
  cancelled() {
    return app.cancel;
  },
  wantsDevice(vid, pid, what) {
    $("grant-what").textContent =
      `The speaker restarted into ${what}: it is a USB device of its own (${hex4(vid)}:${hex4(pid)}).`;
    $("grant").dataset.vid = vid;
    $("grant").dataset.pid = pid;
    if (!$("grant").open) $("grant").showModal(); // asked again each minute until it is granted
  },
  backup(tar, name, holds) {
    return keepBackup(tar, name, holds);
  },
  console(b) {
    consoleText(b);
  },
};

export function appendLog(s) {
  const l = $("log");
  const end = l.scrollTop + l.clientHeight >= l.scrollHeight - 4;
  l.textContent = (l.textContent + s).slice(-60000);
  if (end) l.scrollTop = l.scrollHeight;
}
// ---- connecting ---------------------------------------------------------------------------------------------------

export async function connect(ask) {
  if (ask) {
    try {
      await navigator.usb.requestDevice({
        filters: [
          { vendorId: app.S.usb.vid, productId: app.S.usb.pid },
          { vendorId: app.S.usb.vid, productId: app.S.usb.dfuPid },
        ],
      });
    } catch {
      return;
    } // the chooser closed
  }
  try {
    app.sp = await Speaker.connect(events);
    app.bl = app.sp ? null : await Bootloader.connect(events);
  } catch (e) {
    failed(e);
    return;
  }
  $("press").close();
  if (app.sp) {
    if (app.sp.hello().proto !== app.S.proto) {
      notice(
        `This firmware speaks host protocol ${app.sp.hello().proto}, this page ${app.S.proto}: only its firmware can be ` +
          "installed from here (Software).",
        "warn",
      );
    }
    show("app");
    await refresh(true);
  } else if (app.bl) {
    show("dfu");
    await drawDfu();
  } else show("welcome");
}

/// The speaker over the network at `addr` (docs/protocol/net.md): this browser's key (made once), the speaker's key
/// from the first time at this address (another one there: not the speaker this browser paired with).
export async function connectNet(addr) {
  addr = addr.trim();
  if (!addr) return;
  let me = localStorage.getItem("zd1.identity");
  if (!me) {
    me = newIdentity();
    localStorage.setItem("zd1.identity", me);
  }
  const known = JSON.parse(localStorage.getItem("zd1.speakers") ?? "{}");
  try {
    app.sp = await Speaker.connectNet(addr, me, known[addr], events);
  } catch (e) {
    if (known[addr] && /another key/.test(e?.message ?? "")) {
      const again = await ask(
        "Another speaker at this address",
        `The speaker answering at ${addr} isn't the one this browser paired with there (its key changed: another ` +
          "speaker, or this one's data was erased). Forget the old one and pair with this one? That takes a press of " +
          "ON/OFF on it.",
        "Pair with it",
      );
      if (again) {
        delete known[addr];
        localStorage.setItem("zd1.speakers", JSON.stringify(known));
        return connectNet(addr);
      }
    }
    failed(e);
    return;
  }
  if (!known[addr]) {
    known[addr] = app.sp.speakerKey();
    localStorage.setItem("zd1.speakers", JSON.stringify(known));
  }
  localStorage.setItem("zd1.address", addr);
  app.bl = null;
  show("app");
  await refresh(true);
}

/// Over the network, after changes that can cut the page off (`provisional`): connect again once they have taken
/// effect, at this address or `also` (one a change set); the speaker keeps them then, else it goes back by itself.
export async function reachAgain(also) {
  if (app.sp?.link() !== "net") return;
  notice("The speaker applies the change: this page connects again to keep it…");
  try {
    const at = await app.sp.reachAgain(also);
    localStorage.setItem("zd1.address", at);
    notice(`The speaker answers at ${at}: the change is kept.`);
  } catch (e) {
    app.sp = null;
    show("welcome");
    failed(e);
  }
}

/// "welcome", "app" (our firmware) or "dfu" (the bootloader). The release rows go with the view.
export function show(what) {
  document.body.classList.toggle("net", what === "app" && app.sp?.link() === "net");
  $("welcome").hidden = what !== "welcome";
  $("app").hidden = what !== "app";
  $("dfu").hidden = what !== "dfu";
  $("disconnect").hidden = what === "welcome";
  $("playing").hidden = what !== "app";
  if (what === "welcome") hero({ mode: what, title: "Zeppelin Air", status: "Not connected" });
  if (what === "dfu") hero({ mode: what, title: "Bootloader", status: "Waiting for firmware" });
  if (what === "app") $("verdict").after($("builds"));
}

/// One operation at a time from the page's side (the WebAssembly queues them too); the press dialog closes after.
export async function op(f) {
  app.cancel = false;
  app.busy++;
  document.body.classList.add("busy");
  try {
    return await f();
  } catch (e) {
    failed(e);
  } finally {
    $("press").close();
    if (--app.busy === 0) document.body.classList.remove("busy");
  }
}
/// An operation, then the speaker drawn again.
export const act = (f, all = true) =>
  op(f).then((r) => {
    refresh(all);
    return r;
  });

// ---- the job strip: one long flow at a time, its phases, its outcome --------------------------------------------

/// `f` runs as the job; its result (a string) is the outcome. A connection lost on the way says what to do.
export async function job(title, stoppable, f) {
  if (jobRunning) return notice("Another job runs: wait for it to end.", "warn");
  jobRunning = true;
  app.cancel = false;
  $("job").hidden = false;
  $("job").className = "job";
  $("job-title").textContent = title;
  $("job-phase").textContent = "";
  $("job-now").textContent = "";
  $("job-bar").value = 0;
  heroProgress(0);
  $("job-log").textContent = "";
  $("job-outcome").textContent = "";
  $("job-cancel").hidden = !stoppable;
  $("job-close").hidden = true;
  $("update").hidden = true; // the banner's action is the job now: drawn again (or not) from the state after it
  app.busy++;
  document.body.classList.add("busy");
  try {
    const r = await f();
    $("job-outcome").textContent = typeof r === "string" ? r : "Done.";
    $("job").classList.add("ok");
    return r ?? true;
  } catch (e) {
    $("job").classList.add("failed");
    $("job-outcome").textContent =
      e?.code === "CANCELLED"
        ? "Stopped."
        : e?.code === "DENIED"
          ? "Nobody pressed ON/OFF on the speaker in time: nothing changed."
          : `${e?.message ?? e}${
              e?.code === "GONE"
                ? " The speaker's connection went away: plug it in again and connect; the page shows the state it is in and the way out (in its bootloader: Repair, or a backup)."
                : e?.code === "DEVICE"
                  ? " The browser couldn't open the speaker: connect again (or unplug it and plug it in first); the page shows the state it is in and what is left to do."
                  : ""
            }`;
  } finally {
    jobRunning = false;
    $("press").close();
    $("job-cancel").hidden = true;
    $("job-close").hidden = false;
    $("job-bar").value = 1;
    heroProgress(1);
    setTimeout(() => jobRunning || heroProgress(null), 1500);
    if (--app.busy === 0) document.body.classList.remove("busy");
    app.drawnFor = ""; // the releases drawn again at the next refresh: the banner back only if something is behind
  }
}
