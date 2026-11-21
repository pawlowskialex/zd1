// BL2's web updater, as the page talks to it (docs/protocol/net.md, The ZD1L's first install): no imports, so node runs
// it against the emulator's BL2 too (host/web/test/zd1l.mjs).

/// The confirm's button value (BL2's aspSetFirmwareUploadConfirmationHandler), form-encoded.
const OK = "++++++Ok++++++";

/// BL2's upload: prepare (it arms the upload), the file (its 302 to the apply page, which the browser follows: that page
/// arms the confirm, without it the confirm burns nothing), the expert page, the expert confirm (it starts the burn: the
/// MCU's part first, through the MCU's bootloader, then the module's).
export async function bl2Upload(addr, fw, name, mode = "no-cors") {
  const at = (path, init = {}) => fetch(`http://${addr}${path}`, { mode, cache: "no-store", ...init });
  await at("/1000/firmware_status.asp"); // fails only when nothing answers at the address
  await at("/1000/firmware_prepare.asp");
  const form = new FormData();
  form.append("appFirmware", new Blob([fw]), name);
  form.append("appFirmwareFile", "Upload");
  await at("/goform/formPostHandler", { method: "POST", body: form });
  await at("/1000/firmware_expert.asp");
  await at(`/goform/formExpertDownloadConfirmationHandler?button=${OK}`);
}
