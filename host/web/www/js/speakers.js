// The speakers the page can reach on the network, listed under Connect: those this browser paired with (their keys
// kept by connection.js, under their address) and, where the host can look (zd1-app: mDNS), those announcing
// themselves on this network. A row each, by address; a click connects.
import { connectNet, op } from "./connection.js";
import { host } from "./host.js";
import { $, el } from "./ui.js";

const names = () => JSON.parse(localStorage.getItem("zd1.names") ?? "{}");

/// The speaker's name as last seen at `addr`, for its row.
export function rememberName(addr, name) {
  if (!name) return;
  localStorage.setItem("zd1.names", JSON.stringify({ ...names(), [addr]: name }));
}

let drawn = "";

/// The list drawn again (when what it shows changed: a click isn't lost to a redraw).
export async function drawSpeakers() {
  const known = Object.keys(JSON.parse(localStorage.getItem("zd1.speakers") ?? "{}"));
  const named = names();
  const found = host.speakers ? await host.speakers().catch(() => []) : [];
  const rows = new Map();
  for (const f of found) rows.set(f.address, { addr: f.address, name: f.name, found: true });
  for (const k of known) rows.set(k, { name: named[k], ...rows.get(k), addr: k, paired: true });
  const list = [...rows.values()];
  const key = JSON.stringify(list);
  if (key === drawn) return;
  drawn = key;
  const items = list.map((s) => {
    const r = el(
      "button",
      { className: "row item", type: "button" },
      el("strong", { textContent: s.name || s.addr }),
      s.name ? el("span", { className: "small data", textContent: s.addr }) : null,
      el("span", { className: "grow" }),
      s.paired ? el("span", { className: "tag", textContent: "paired" }) : null,
      host.speakers && !s.found ? el("span", { className: "small", textContent: "not seen now" }) : null,
    );
    r.onclick = () => op(() => connectNet(s.addr));
    return r;
  });
  if (host.speakers && !list.length)
    items.push(
      el("p", {
        className: "small",
        textContent:
          "Looking for speakers on this network… One whose AirPlay is on shows here; else its address below " +
          "(the Network panel shows it once connected over USB).",
      }),
    );
  $("net-list").replaceChildren(...items);
}

/// The list kept up to date while the Connect panel shows (zd1-app: what mDNS sees comes and goes).
export function watchSpeakers() {
  drawSpeakers();
  if (host.speakers) setInterval(() => $("welcome").hidden || drawSpeakers(), 3000);
}
