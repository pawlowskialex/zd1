// The page's WebAssembly over the network (docs/protocol/net.md), in node: its `Speaker.connectNet` through node's own
// WebSocket to the emulator's forwarded port. ON/OFF held 5 s must come while the first request waits for it (the
// pairing): zemu's --confirm.
//   zemu run --mcu --fwd 18080:80 --confirm 5 --host 'node host/web/test/net.mjs "$(nix build --no-link --print-out-paths .#web)" 127.0.0.1:18080'
// With SIGNED=FILE (a signed system image, `zd1 sign`) the speaker also gets it (it needs slots: `zemu run --nand`).
import fs from "node:fs";
import path from "node:path";

const [dirArg, addr] = process.argv.slice(2);
if (!dirArg || !addr) {
  console.error("usage: node net.mjs PAGE_DIR HOST:PORT");
  process.exit(2);
}
const dir = path.resolve(dirArg);
const wasm = await import(path.join(dir, "pkg", "zd1_web.js"));
wasm.initSync({ module: fs.readFileSync(path.join(dir, "pkg", "zd1_web_bg.wasm")) });
let prompted = "";
const events = {
  text() {},
  prompt: (how) => {
    prompted = how;
  },
  step: (s) => console.log("step:", s),
  phase() {},
  progress() {},
  wait() {},
  cancelled: () => false,
  wantsDevice() {},
  backup() {},
  console() {},
};
let fails = 0;
const check = (ok, what) => {
  console.log(ok ? "ok  " : "FAIL", what);
  if (!ok) fails++;
};
const refused = async (f, re, what) => {
  try {
    await f();
    check(false, what);
  } catch (e) {
    check(re.test(e.message), `${what}: ${e.message}`);
  }
};

const me = wasm.newIdentity();
const sp = await wasm.Speaker.connectNet(addr, me, undefined, events);
const key = sp.speakerKey();
check(
  sp.link() === "net" && /^[0-9a-f]{64}$/.test(key),
  `connected over the network: the speaker's key ${key.slice(0, 8)}`,
);
const h = sp.hello();
check(typeof h.fw === "string" && h.requests.length === 32, `hello before pairing: ${h.fw}`);
const info = await sp.call("Info", {});
check(prompted === "pair" && info.msg === "InfoReply", `paired with the hold, then Info: module ${info.mod_os}`);
const st = await sp.state();
check(st.msg === "State" && typeof st.name === "string", `state: ${st.os}, slot ${st.slot}`);
const paired = await sp.list("Controllers");
check(
  paired.some((c) => c.name === "the web page"),
  `paired: ${paired.map((c) => `${c.name} (${c.id})`).join(", ")}`,
);
const name = `net test ${Date.now() % 1000}`;
check((await sp.setSetting("name", name)) === name, "a setting over the network");
check((await sp.call("GetSetting", { key: "name" })).value === name, "read back");
await sp.setSetting("name", "");
const v = await sp.call("Volume", { volume: 12, bass: 1 });
check(v.msg === "Ok" && (await sp.call("Info", {})).volume === 12, "the volume (the MCU's, through the module)");
await refused(() => sp.consoleOpen(true), /network/, "the console stays USB's");
await refused(() => sp.call("Peek", { addr: 0x80000000, len: 4, words: false }), /network/, "Peek stays USB's");
await refused(() => sp.setSetting("net", "off"), /USB only/, "the network link is switched over USB only");
check(wasm.provisional("eth.dns") && !wasm.provisional("name"), "which settings are provisional");
await sp.setSetting("eth.dns", "192.0.2.53");
const at = await sp.reachAgain(undefined);
check(
  at === addr && (await sp.call("GetSetting", { key: "eth.dns" })).value === "192.0.2.53",
  `reached again at ${at}: kept`,
);
await sp.setSetting("eth.dns", "");
await sp.reachAgain(undefined);
await refused(
  () => wasm.Speaker.connectNet(addr, me, "00".repeat(32), events),
  /another key/,
  "another key at this address refused",
);
// another browser, nobody at the speaker: refused after the press's 30 s (asked once: it gives up then)
const other = await wasm.Speaker.connectNet(addr, wasm.newIdentity(), key, { ...events, cancelled: () => true });
await refused(() => other.call("BootState", {}), /stopped|nobody/i, "an unpaired browser: nothing without the hold");
other.free();
if (process.env.SIGNED) {
  const img = fs.readFileSync(process.env.SIGNED);
  check(wasm.imageInfo("system.signed", img).kind === "signed", "a signed image");
  const r = await sp.updateSystem(img, true);
  check(r.outcome === "running", `updated over the network: ${r.outcome}, slot ${r.slot}`);
  const st2 = await sp.state();
  check(st2.slot === st2.good, `the session is back: slot ${st2.slot}, good ${st2.good}`);
}
sp.free();
process.exit(fails ? 1 : 0);
