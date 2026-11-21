// zd1-app's part of the page (host/app/src/main.rs), before the page's own scripts: `navigator.usb` with the
// speaker's devices granted, and a WebSocket and fetch that reach the local network, each call a request to the app
// (wry's `window.ipc`) answered by `__zd1.done`, events from it by `__zd1.event`. Only what the page's nusb and its
// scripts use is here; the rest of the web view is the system's.
(() => {
  "use strict";
  if (window.__zd1) return;
  const LOG = false; // ZD1_APP_LOG (the app sets it)

  const pending = new Map();
  let nextId = 1;
  const call = (op, args = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ ...args, id, op }));
    });
  const listeners = {};

  const b64 = (u8) => {
    let s = "";
    for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
    return btoa(s);
  };
  const unb64 = (s) => {
    const bin = atob(s ?? "");
    const u8 = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);
    return u8;
  };
  const bytes = (d) =>
    d instanceof ArrayBuffer ? new Uint8Array(d) : new Uint8Array(d.buffer, d.byteOffset, d.byteLength);

  window.__zd1 = {
    done(id, ok, v) {
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(v);
      else p.reject(v.name === "TypeError" ? new TypeError(v.message) : new DOMException(v.message, v.name));
    },
    event(kind, data) {
      listeners[kind]?.(data);
    },
  };

  // ---- WebUSB ---------------------------------------------------------------------------------------------------

  class USBInTransferResult {
    constructor(status, data) {
      this.status = status;
      this.data = data;
    }
  }
  class USBOutTransferResult {
    constructor(status, bytesWritten) {
      this.status = status;
      this.bytesWritten = bytesWritten;
    }
  }
  class USBConnectionEvent extends Event {
    constructor(type, init) {
      super(type, init);
      this.device = init.device;
    }
  }
  const inResult = (r) => new USBInTransferResult(r.status, new DataView(unb64(r.data).buffer));
  const outResult = (r) => new USBOutTransferResult(r.status, r.bytesWritten);
  const configuration = (c) => ({
    ...c,
    interfaces: c.interfaces.map((i) => ({ ...i, alternate: i.alternates[0] ?? null, claimed: false })),
  });

  class USBDevice {
    #key;
    constructor(info) {
      this.#key = info.key;
      this.opened = false;
      this._update(info);
    }
    _update(i) {
      const claimed = new Set(
        (this.configuration?.interfaces ?? []).filter((f) => f.claimed).map((f) => f.interfaceNumber),
      );
      const v = (bcd, part) => [bcd >> 8, (bcd >> 4) & 15, bcd & 15][part];
      Object.assign(this, {
        vendorId: i.vendorId,
        productId: i.productId,
        deviceClass: i.deviceClass,
        deviceSubclass: i.deviceSubclass,
        deviceProtocol: i.deviceProtocol,
        usbVersionMajor: v(i.usb, 0),
        usbVersionMinor: v(i.usb, 1),
        usbVersionSubminor: v(i.usb, 2),
        deviceVersionMajor: v(i.device, 0),
        deviceVersionMinor: v(i.device, 1),
        deviceVersionSubminor: v(i.device, 2),
        manufacturerName: i.manufacturerName ?? null,
        productName: i.productName ?? null,
        serialNumber: i.serialNumber ?? null,
      });
      this.configurations = i.configurations.map(configuration);
      const active = i.configuration?.configurationValue;
      this.configuration = i.configuration
        ? (this.configurations.find((c) => c.configurationValue === active) ?? configuration(i.configuration))
        : null;
      for (const f of this.configuration?.interfaces ?? []) f.claimed = claimed.has(f.interfaceNumber);
    }
    #call(op, args = {}) {
      return call(`usb.${op}`, { ...args, key: this.#key });
    }
    #intf(n) {
      const i = this.configuration?.interfaces.find((f) => f.interfaceNumber === n);
      if (!i) throw new DOMException(`no interface ${n}`, "NotFoundError");
      return i;
    }
    async open() {
      this._update(await this.#call("open"));
      this.opened = true;
    }
    async close() {
      await this.#call("close");
      this.opened = false;
      for (const f of this.configuration?.interfaces ?? []) f.claimed = false;
    }
    async forget() {
      await this.#call("forget");
    }
    async selectConfiguration(value) {
      this._update(await this.#call("selectConfiguration", { value }));
    }
    async claimInterface(n) {
      const i = this.#intf(n);
      await this.#call("claimInterface", { interface: n });
      i.claimed = true;
    }
    async releaseInterface(n) {
      const i = this.#intf(n);
      await this.#call("releaseInterface", { interface: n });
      i.claimed = false;
    }
    async selectAlternateInterface(n, alternate) {
      const i = this.#intf(n);
      await this.#call("selectAlternateInterface", { interface: n, alternate });
      i.alternate = i.alternates.find((a) => a.alternateSetting === alternate) ?? i.alternate;
    }
    async controlTransferIn(setup, length) {
      return inResult(await this.#call("controlTransferIn", { setup, length }));
    }
    async controlTransferOut(setup, data) {
      return outResult(await this.#call("controlTransferOut", { setup, data: data ? b64(bytes(data)) : null }));
    }
    async transferIn(endpoint, length) {
      return inResult(await this.#call("transferIn", { endpoint, length }));
    }
    async transferOut(endpoint, data) {
      return outResult(await this.#call("transferOut", { endpoint, data: b64(bytes(data)) }));
    }
    async clearHalt(direction, endpoint) {
      await this.#call("clearHalt", { direction, endpoint });
    }
    async reset() {
      await this.#call("reset");
    }
    async isochronousTransferIn() {
      throw new DOMException("isochronous transfers aren't bridged", "NotSupportedError");
    }
    async isochronousTransferOut() {
      throw new DOMException("isochronous transfers aren't bridged", "NotSupportedError");
    }
  }

  const devices = new Map(); // key -> USBDevice: one object per device, as a browser keeps it
  const device = (info) => {
    let d = devices.get(info.key);
    if (d) d._update(info);
    else devices.set(info.key, (d = new USBDevice(info)));
    return d;
  };
  const matches = (f, d) =>
    (f.vendorId === undefined || f.vendorId === d.vendorId) &&
    (f.productId === undefined || f.productId === d.productId) &&
    (f.classCode === undefined || f.classCode === d.deviceClass) &&
    (f.subclassCode === undefined || f.subclassCode === d.deviceSubclass) &&
    (f.protocolCode === undefined || f.protocolCode === d.deviceProtocol) &&
    (f.serialNumber === undefined || f.serialNumber === d.serialNumber);

  class USB extends EventTarget {
    onconnect = null;
    ondisconnect = null;
    async getDevices() {
      const here = await call("usb.list");
      for (const k of [...devices.keys()]) if (!here.some((i) => i.key === k)) devices.delete(k);
      return here.map(device);
    }
    /// Granted already: the first device here that a filter matches, no chooser.
    async requestDevice(options) {
      if (!Array.isArray(options?.filters)) throw new TypeError("requestDevice needs filters");
      const d = (await this.getDevices()).find((d) => options.filters.some((f) => matches(f, d)));
      if (!d) throw new DOMException("No device selected.", "NotFoundError");
      return d;
    }
    _fire(type, d) {
      const e = new USBConnectionEvent(type, { device: d });
      this.dispatchEvent(e);
      this[`on${type}`]?.call(this, e);
    }
  }
  const usb = new USB();
  listeners["usb.connect"] = (info) => usb._fire("connect", device(info));
  listeners["usb.disconnect"] = ({ key }) => {
    const d = devices.get(key);
    if (!d) return;
    devices.delete(key);
    d.opened = false;
    usb._fire("disconnect", d);
  };
  Object.defineProperty(Navigator.prototype, "usb", { get: () => usb, configurable: true, enumerable: true });
  Object.assign(window, { USB, USBDevice, USBConnectionEvent, USBInTransferResult, USBOutTransferResult });

  // ---- the local network: WebSocket and fetch --------------------------------------------------------------------

  const local = (host) => {
    const h = host.toLowerCase().replace(/^\[|\]$/g, "");
    return (
      h === "localhost" ||
      (h.endsWith(".local") && h.length > 6) ||
      /^(127|10)\./.test(h) ||
      /^192\.168\./.test(h) ||
      /^172\.(1[6-9]|2\d|3[01])\./.test(h) ||
      /^169\.254\./.test(h) ||
      h === "::1" ||
      /^f[cd][0-9a-f]{0,2}:/.test(h) ||
      /^fe[89ab][0-9a-f]?:/.test(h)
    );
  };

  const NativeWebSocket = window.WebSocket;
  const sockets = new Map();
  let nextSock = 1;
  class WebSocket extends EventTarget {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;
    #sock;
    #sending = Promise.resolve();
    constructor(url, protocols) {
      const u = new URL(url, location.href);
      if (u.protocol !== "ws:" || !local(u.hostname)) return new NativeWebSocket(url, protocols);
      super();
      Object.assign(this, { url: u.href, readyState: 0, binaryType: "blob", protocol: "", extensions: "" });
      Object.assign(this, { bufferedAmount: 0, onopen: null, onmessage: null, onclose: null, onerror: null });
      this.#sock = nextSock++;
      sockets.set(this.#sock, this);
      call("net.wsOpen", { sock: this.#sock, url: u.href }).catch((e) => {
        this._event({ type: "error" });
        this._event({ type: "close", code: 1006, reason: String(e.message), clean: false });
      });
    }
    get CONNECTING() {
      return 0;
    }
    get OPEN() {
      return 1;
    }
    get CLOSING() {
      return 2;
    }
    get CLOSED() {
      return 3;
    }
    send(data) {
      if (this.readyState === 0) throw new DOMException("Still in CONNECTING state.", "InvalidStateError");
      if (this.readyState !== 1) return;
      // the bytes now, as a browser takes them: a view into WebAssembly's memory changes once send returns; in
      // order, a Blob's bytes too
      const now =
        typeof data === "string" ? { text: data } : data instanceof Blob ? null : { data: b64(bytes(data)) };
      this.#sending = this.#sending.then(async () => {
        const args = now ?? { data: b64(new Uint8Array(await data.arrayBuffer())) };
        await call("net.wsSend", { sock: this.#sock, ...args }).catch(() => {});
      });
    }
    close(code = 1000, reason = "") {
      if (this.readyState >= 2) return;
      this.readyState = 2;
      this.#sending.then(() => call("net.wsClose", { sock: this.#sock, code, reason })).catch(() => {});
    }
    _event(e) {
      let ev;
      if (e.type === "open") {
        this.readyState = 1;
        ev = new Event("open");
      } else if (e.type === "message") {
        const data =
          e.text !== undefined ? e.text : this.binaryType === "arraybuffer" ? unb64(e.data).buffer : new Blob([unb64(e.data)]);
        ev = new MessageEvent("message", { data, origin: new URL(this.url).origin });
      } else if (e.type === "close") {
        if (this.readyState === 3) return;
        this.readyState = 3;
        sockets.delete(this.#sock);
        ev = new CloseEvent("close", { code: e.code, reason: e.reason, wasClean: e.clean });
      } else ev = new Event("error");
      this.dispatchEvent(ev);
      this[`on${ev.type}`]?.call(this, ev);
    }
  }
  listeners.ws = (e) => sockets.get(e.sock)?._event(e);
  window.WebSocket = WebSocket;

  const nativeFetch = window.fetch;
  window.fetch = async function (input, init) {
    const req = new Request(input, init);
    const u = new URL(req.url);
    // the page's own origin, and anything not plain HTTP to the local network, stays the web view's
    if (u.origin === location.origin || u.protocol !== "http:" || !local(u.hostname))
      return nativeFetch.call(this, input, init);
    const abort = () => req.signal.reason ?? new DOMException("The operation was aborted.", "AbortError");
    if (req.signal.aborted) throw abort();
    const body = req.method === "GET" || req.method === "HEAD" ? null : new Uint8Array(await req.arrayBuffer());
    // the answer in full whatever the mode: a browser's no-cors would hide it (the app has no cross-origin to guard)
    const r = await Promise.race([
      call("net.fetch", { url: req.url, method: req.method, headers: [...req.headers], body: body && b64(body) }),
      new Promise((_, reject) => req.signal.addEventListener("abort", () => reject(abort()), { once: true })),
    ]);
    const empty = req.method === "HEAD" || [101, 204, 205, 304].includes(r.status);
    const res = new Response(empty ? null : unb64(r.body), {
      status: r.status,
      statusText: r.statusText,
      headers: r.headers,
    });
    // where the redirects led, as a browser's fetch tells it
    Object.defineProperty(res, "url", { value: r.url });
    Object.defineProperty(res, "redirected", { value: new URL(r.url).href !== u.href });
    return res;
  };

  // ---- what the app adds (the page's js/host.js asks for these by name) -----------------------------------------
  window.zd1app = Object.freeze({
    /// the speaker's USB devices are granted from the start: no chooser, and one plugged in is there at once
    granted: true,
    /// plain HTTP and WebSockets reach the local network without a permission, and fetch reads every answer
    lan: true,
    /// our speakers on the network (mDNS): [{ name, address, host }]
    speakers: () => call("net.speakers"),
    /// this computer's IPv4 networks: [{ interface, address, prefix }]
    networks: () => call("net.networks"),
  });

  // ---- what the page logs, to the app's stderr (ZD1_APP_LOG) -------------------------------------------------------
  if (LOG) {
    const out = (level, args) => {
      const text = args.map((a) => (a instanceof Error ? `${a.name}: ${a.message}` : typeof a === "string" ? a : JSON.stringify(a))).join(" ");
      window.ipc.postMessage(JSON.stringify({ op: "log", text: `${level}: ${text}` }));
    };
    for (const level of ["log", "warn", "error"]) {
      const orig = console[level].bind(console);
      console[level] = (...a) => {
        orig(...a);
        try {
          out(level, a);
        } catch {}
      };
    }
    window.addEventListener("error", (e) => out("uncaught", [e.message, `${e.filename}:${e.lineno}`]));
    window.addEventListener("unhandledrejection", (e) => out("unhandled", [e.reason]));
  }
})();
