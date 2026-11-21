// What runs the page. A browser gives it the web platform only: WebUSB behind its chooser, the local network behind
// its permission (Chrome's Local Network Access) and BL2's answers hidden (no-cors). zd1-app (host/app: the page in a
// window of its own) grants the speaker's USB devices from the start, lets it reach the local network and read every
// answer, and adds `window.zd1app` for what no browser offers. The page asks here, by what it needs, never by name.

const a = window.zd1app;

export const host = {
  /// The speaker's USB devices granted already: no chooser, and a speaker plugged in connects by itself.
  granted: !!a?.granted,
  /// The local network reachable without a permission to ask for.
  lan: !!a?.lan,
  /// Our speakers on the network (mDNS): [{ name, address, host }]; null where nothing can look (a browser).
  speakers: a?.speakers ?? null,
  /// This computer's IPv4 networks: [{ interface, address, prefix }]; null in a browser.
  networks: a?.networks ?? null,
};
