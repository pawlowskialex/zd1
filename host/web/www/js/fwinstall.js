// The first install on a model whose BL2 takes a B&W .fw, from the page (docs/protocol/net.md, The first install on a
// ZD1L or an A7): the release's .fw for that model to the updater its network module runs from key-hold (BL2's web
// updater), with the requests bl2web sends (tools/zeppelin/knos/bl2web.py, seen working on ZD1Ls). BL2 sends no CORS
// headers, so the browser lets the page send them but read none of the answers ("no-cors"): no progress, no refusal.
// The page then waits for our software to answer at the address.
import { newIdentity, Speaker } from "../pkg/zd1_web.js";
import { bl2Upload } from "./bl2.js";
import { connectNet, events, job } from "./connection.js";
import { FW_MODELS, fetchImage, fwRelease } from "./software.js";
import { ask, notice } from "./ui.js";

/// Our software at `addr`: its network link answers the session's handshake.
async function ours(addr) {
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

export async function installFw(model, addr) {
  addr = addr.trim();
  if (!addr) return;
  const m = FW_MODELS[model],
    found = fwRelease(model);
  if (!found) return notice(`No release offers ${m.name}'s file to this page.`, "warn");
  const { rel, img } = found;
  if (
    !(await ask(
      `Install ${rel.version} on ${m.name}?`,
      `${m.status} The release's file goes to the speaker's updater at ${addr}, which installs it on both of its ` +
        "chips and restarts it with our software: about ten minutes. Keep its power and its cable in meanwhile. If " +
        `it doesn't come up, B&W's own file (${m.stock}) goes back on the same way.`,
      "Install",
    ))
  )
    return;
  const title = m.name[0].toUpperCase() + m.name.slice(1);
  const done = await job(`${title}: ${rel.version}`, false, async () => {
    events.phase(1, 3, "the release's file");
    const fw = await fetchImage(img);
    events.phase(2, 3, `to the speaker's updater at ${addr}`);
    try {
      await bl2Upload(addr, fw, img.file.split("/").pop());
    } catch {
      throw new Error(
        `Nothing answers at ${addr}: is the speaker's light white (its updater), its cable in, and the address the one ` +
          "your router gave it? The browser may also have been refused access to the local network (it asks once).",
      );
    }
    events.phase(3, 3, "the speaker installs it and starts");
    const t0 = Date.now();
    while (Date.now() - t0 < 15 * 60 * 1000) {
      events.wait("our software answering", Math.round((Date.now() - t0) / 1000));
      await new Promise((r) => setTimeout(r, 10000));
      if (await ours(addr)) return `Installed: our software answers at ${addr}.`;
    }
    throw new Error(
      `Nothing of ours answered at ${addr} within 15 minutes. If the light is still white, the updater refused the ` +
        "file (it checks the model); if it plays, it may have taken another address (the router's list).",
    );
  });
  if (done) await connectNet(addr);
}
