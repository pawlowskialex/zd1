// The page's upload to a ZD1L's BL2 (js/bl2.js: the ZD1L's first install, docs/protocol/net.md), in node against the
// emulator's BL2. Node reads the answers the browser can't (no CORS headers): every request answered, the upload's
// redirect to the apply page followed, the confirm armed (it redirects to the status page; unarmed it goes to
// FWUCHandlerPage and burns nothing). A stock ZD1L held at power-up (emu/tools/zd1lunit.py):
//   zemu run OUT/nand.bin --nand --mcu OUT/mcu_flash.bin --fwd 18080:80 --renode 'zd1:flashc BootprotSize 0x8000' \
//     --renode 'zd1:board Adc "2:0x300"' --renode 'zd1:board Pins "0:PA08=0,1500:PA08=1"' \
//     --wait mcu 'BSL Ready' --sleep 30 --host 'node host/web/test/zd1l.mjs host/web/www 127.0.0.1:18080 zd1l.fw' \
//     --wait mcu 'ReceiveHostFirmware\(\) - (Done|Failed)' ...
import fs from "node:fs";
import path from "node:path";

const [dirArg, addr, file] = process.argv.slice(2);
if (!dirArg || !addr || !file) {
  console.error("usage: node zd1l.mjs PAGE_DIR HOST:PORT FILE.fw");
  process.exit(2);
}
const { bl2Upload } = await import(path.join(path.resolve(dirArg), "js", "bl2.js"));
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
await bl2Upload(addr, fs.readFileSync(file), path.basename(file), "cors");
const bad = [];
if (seen.some((s) => s.status !== 200)) bad.push("a request not answered 200");
if (seen[2]?.to !== "/1000/firmware_apply.asp") bad.push("the upload didn't lead to the apply page");
if (seen[4]?.to !== "/1000/firmware_status.asp")
  bad.push(`the confirm went to ${seen[4]?.to || "no page"}, not the status page: not armed`);
if (bad.length) {
  console.log(`FAIL ${bad.join("; ")}`);
  process.exit(1);
}
console.log("ok   BL2 took the file and the confirm: it burns both chips now");
