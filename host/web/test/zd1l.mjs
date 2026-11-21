// The page's side of a ZD1L's BL2 (js/bl2.js: the Lightning model's install and the way back, docs/protocol/net.md),
// in node against the emulator's BL2. Node reads the answers a browser can't (no CORS headers), as zd1-app does: what
// answers (bl2Look), every request answered, the upload's redirect to the apply page followed, the confirm armed (it
// redirects to the status page; unarmed it goes to FWUCHandlerPage and burns nothing), then the write followed on the
// status page to its end (bl2Burn). A stock ZD1L held at power-up (emu/tools/zd1lunit.py):
//   zemu run OUT/nand.bin --nand --mcu OUT/mcu_flash.bin --fwd 18080:80 --renode 'zd1:flashc BootprotSize 0x8000' \
//     --renode 'zd1:board Adc "2:0x300"' --renode 'zd1:board Pins "0:PA08=0,1500:PA08=1"' \
//     --wait mcu 'BSL Ready' --sleep 30 --host 'node host/web/test/zd1l.mjs host/web/www 127.0.0.1:18080 zd1l.fw'
import fs from "node:fs";
import path from "node:path";

const [dirArg, addr, file] = process.argv.slice(2);
if (!dirArg || !addr || !file) {
  console.error("usage: node zd1l.mjs PAGE_DIR HOST:PORT FILE.fw");
  process.exit(2);
}
const { bl2Burn, bl2Look, bl2Upload } = await import(path.join(path.resolve(dirArg), "js", "bl2.js"));
const look = await bl2Look(addr, { mode: "cors" });
console.log("look:", JSON.stringify(look));
if (look.kind !== "zd1l") {
  console.log("FAIL the Lightning model's updater doesn't answer");
  process.exit(1);
}
const seen = [];
const real = globalThis.fetch;
globalThis.fetch = async (url, init) => {
  const r = await real(url, init);
  const to = r.redirected ? new URL(r.url).pathname : "";
  console.log(
    `${init?.method ?? "GET"} ${new URL(url).pathname}${url.includes("?") ? "?…" : ""} -> ${r.status}${to ? ` (to ${to})` : ""}`,
  );
  seen.push({ status: r.status, to });
  return r;
};
const read = await bl2Upload(addr, fs.readFileSync(file), path.basename(file), { mode: "cors" });
globalThis.fetch = real; // the status polls unlogged
const bad = [];
if (seen.some((s) => s.status !== 200)) bad.push("a request not answered 200");
if (seen[2]?.to !== "/1000/firmware_apply.asp") bad.push("the upload didn't lead to the apply page");
if (seen[4]?.to !== "/1000/firmware_status.asp")
  bad.push(`the confirm went to ${seen[4]?.to || "no page"}, not the status page: not armed`);
if (!read) bad.push("bl2Upload didn't read the answers");
if (bad.length) {
  console.log(`FAIL ${bad.join("; ")}`);
  process.exit(1);
}
console.log("ok   BL2 took the file and the confirm: it burns both chips now");
const t0 = Date.now();
try {
  await bl2Burn(addr, (s) => console.log(`  ${Math.round((Date.now() - t0) / 1000)} s: ${JSON.stringify(s)}`));
} catch (e) {
  console.log(`FAIL ${e.message}`);
  process.exit(1);
}
console.log(`ok   BL2 says the write finished, both parts (${Math.round((Date.now() - t0) / 1000)} s)`);
