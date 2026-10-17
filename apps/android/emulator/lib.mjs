// Helpers for the Android emulator checks (docs/android-manual-checks.md): adb and the emulator
// console, the UI through `uiautomator dump` and `input`, the app's WebView through Playwright's
// Android support, the home server's API, and the ntfy server UnifiedPush runs through.

import { execFile, execFileSync, spawn } from "node:child_process";
import { createRequire } from "node:module";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { networkInterfaces } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export const repo = fileURLToPath(new URL("../../..", import.meta.url));

/** The APK under test: THEVAULT_APK, or the x86_64 debug APK `just android-build x86_64` writes. */
export const APK = process.env.THEVAULT_APK ?? join(repo, "apps/android/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");

/** `aapt2 dump badging` of `apk`: its package id, its version and whether it is a debuggable build. */
export function badging(apk) {
  const tools = join(process.env.ANDROID_HOME ?? "", "build-tools");
  const versions = existsSync(tools) ? readdirSync(tools).sort((a, b) => a.localeCompare(b, undefined, { numeric: true })) : [];
  const aapt2 = versions.length ? join(tools, versions.at(-1), "aapt2") : "aapt2";
  const out = execFileSync(aapt2, ["dump", "badging", apk], { encoding: "utf8", maxBuffer: 64 << 20 });
  const pkg = /^package: name='([^']+)'/m.exec(out)?.[1];
  if (!pkg) throw new Error(`aapt2 dump badging ${apk} names no package`);
  return { pkg, debuggable: /^application-debuggable/m.test(out), version: /versionName='([^']*)'/.exec(out)?.[1] ?? "" };
}

// The package id comes from the APK: dev.wiltaylor.thevault.debug for a debug build,
// dev.wiltaylor.thevault for a release build. THEVAULT_PKG names it when the APK is not built.
const badge = existsSync(APK) ? badging(APK) : null;
export const PKG = process.env.THEVAULT_PKG ?? badge?.pkg ?? "dev.wiltaylor.thevault.debug";
/**
 * Whether the app under test is a debuggable build. Only a debuggable build's WebView can be
 * driven with Playwright, since Tauri turns WebView debugging on in debug builds alone; the checks
 * reach a release build's screen through `uiautomator dump` and its vault through the root shell.
 */
export const DEBUGGABLE = badge ? badge.debuggable : PKG.endsWith(".debug");
export const MAIN = `${PKG}/dev.wiltaylor.thevault.MainActivity`;
export const SHARE = `${PKG}/dev.wiltaylor.thevault.device.ShareActivity`;
export const MESSAGES = "com.google.android.apps.messaging";
export const NTFY = "io.heckel.ntfy";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Runs `adb args...` and answers its stdout. */
export function adb(...args) {
  return execFileSync("adb", args, { encoding: "utf8", maxBuffer: 64 << 20, stdio: ["ignore", "pipe", "pipe"] });
}

/** `adb args...`, answering stdout, without throwing on a non-zero exit. */
export function adbTry(...args) {
  try {
    return adb(...args);
  } catch (e) {
    return `${e.stdout ?? ""}${e.stderr ?? ""}`;
  }
}

export const shell = (cmd) => adb("shell", cmd);
export const shellTry = (cmd) => adbTry("shell", cmd);

/** An emulator console command, such as `sms send 5551234 hi` or `gsm call 5551234`. */
export const emu = (cmd) => adb("emu", ...cmd.split(" "));

/** Polls `fn` until it answers something truthy, or throws after `timeout` ms. */
export async function until(what, fn, { timeout = 30_000, every = 500 } = {}) {
  const end = Date.now() + timeout;
  let last;
  for (;;) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    if (Date.now() > end) {
      const why = last instanceof Error ? `: ${last.message}` : "";
      throw new Error(`timed out after ${timeout} ms waiting for ${what}${why}`);
    }
    await sleep(every);
  }
}

// ---------------------------------------------------------------------------------------------
// The screen

/** Every node on screen, from `uiautomator dump`, as { text, desc, id, cls, pkg, checked, enabled, bounds, x, y }. */
export function dump() {
  let xml = "";
  for (let i = 0; i < 3 && !xml.includes("<hierarchy"); i++) {
    xml = shellTry("uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; cat /sdcard/ui.xml");
  }
  const nodes = [];
  for (const m of xml.matchAll(/<node ([^>]*?)\/?>/g)) {
    const a = {};
    for (const p of m[1].matchAll(/([\w-]+)="([^"]*)"/g)) a[p[1]] = unescape(p[2]);
    const b = /\[(\d+),(\d+)\]\[(\d+),(\d+)\]/.exec(a.bounds ?? "");
    if (!b) continue;
    const [x1, y1, x2, y2] = b.slice(1).map(Number);
    nodes.push({
      text: a.text ?? "",
      desc: a["content-desc"] ?? "",
      id: a["resource-id"] ?? "",
      cls: a.class ?? "",
      pkg: a.package ?? "",
      checked: a.checked === "true",
      enabled: a.enabled !== "false",
      bounds: [x1, y1, x2, y2],
      x: Math.round((x1 + x2) / 2),
      y: Math.round((y1 + y2) / 2),
    });
  }
  return nodes;
}

function unescape(s) {
  return s.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#10;/g, "\n").replace(/&amp;/g, "&");
}

/** Whether node `n` matches `want`: a string (its text or description, case-insensitive, whole), a RegExp, or a predicate. */
function matches(n, want) {
  if (typeof want === "function") return want(n);
  if (want instanceof RegExp) return want.test(n.text) || want.test(n.desc);
  const w = want.toLowerCase();
  return n.text.toLowerCase() === w || n.desc.toLowerCase() === w;
}

export const find = (want, nodes = dump()) => nodes.find((n) => matches(n, want));
export const texts = (nodes = dump()) => nodes.map((n) => n.text).filter(Boolean);

/** Waits for a node matching `want`. */
export const waitFor = (want, opts) => until(`"${want}" on screen`, () => find(want), opts);

/** Waits for a node matching `want`, then taps it. */
export async function tap(want, opts) {
  const n = await waitFor(want, opts);
  shell(`input tap ${n.x} ${n.y}`);
  await sleep(400);
  return n;
}

/** Taps `want` if it shows within `timeout` ms; answers whether it did. */
export async function tapIf(want, timeout = 3_000) {
  try {
    await tap(want, { timeout });
    return true;
  } catch {
    return false;
  }
}

/** Types `text` into the focused field. */
export function type(text) {
  // `input text` takes %s for a space and needs shell metacharacters escaped.
  const escaped = text.replace(/[\\'"`$&|;<>()*?~!#]/g, (c) => `\\${c}`).replace(/ /g, "%s");
  shell(`input text ${escaped}`);
}

/** Replaces the text of edit field `node`. */
export async function fill(node, text) {
  shell(`input tap ${node.x} ${node.y}`);
  await sleep(200);
  shell("input keycombination 113 29"); // Ctrl+A
  shell("input keyevent 67"); // Delete
  if (text) type(text);
  await sleep(200);
}

export const key = (code) => shell(`input keyevent ${code}`);
export const back = () => key(4);
export const home = () => key(3);

/** Saves a screenshot as `<dir>/<name>.png` and answers its path. */
export function screenshot(dir, name) {
  mkdirSync(dir, { recursive: true });
  const path = join(dir, `${name}.png`);
  writeFileSync(path, execFileSync("adb", ["exec-out", "screencap", "-p"], { maxBuffer: 64 << 20 }));
  return path;
}

/** The notifications in the shade, from `dumpsys notification`, as { pkg, tag, id, title, text, key }. */
export function notifications() {
  const out = shellTry("dumpsys notification --noredact");
  // A failed dumpsys reads as an empty shade; never take that for "the notification went".
  if (!out.includes("Notification List")) throw new Error("dumpsys notification failed");
  const list = [];
  const records = out.split(/\n\s*NotificationRecord\(/).slice(1);
  for (const r of records) {
    const head = /pkg=(\S+) user=\S+ id=(-?\d+) tag=(\S+) importance=\d+ key=(\S+?):/.exec(r);
    if (!head) continue;
    const title = /android\.title=\S+ \((.*)\)$/m.exec(r)?.[1] ?? "";
    const text = /android\.text=\S+ \((.*)\)$/m.exec(r)?.[1] ?? "";
    list.push({ pkg: head[1], id: Number(head[2]), tag: head[3], key: head[4], title, text });
  }
  return list;
}

export const appNotifications = () => notifications().filter((n) => n.pkg === PKG);

// ---------------------------------------------------------------------------------------------
// Logcat

/** Clears logcat. */
export const logcatClear = () => adbTry("logcat", "-c");

/** Saves the log since the last clear to `path`, and answers its lines matching `pattern`. */
export function logcatSave(path, pattern) {
  const out = adbTry("logcat", "-d", "-v", "time");
  writeFileSync(path, out);
  return pattern ? out.split("\n").filter((l) => pattern.test(l)) : [];
}

// ---------------------------------------------------------------------------------------------
// The home server

/** A free TCP port on the loopback. */
export async function freePort(host = "127.0.0.1") {
  const { createServer } = await import("node:net");
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, host, () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

/** The first line `child` prints, within `timeout` ms. */
export function firstLine(child, what, timeout = 60_000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${what} printed nothing`)), timeout);
    const lines = createInterface({ input: child.stdout });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(line);
      lines.on("line", () => {});
    });
    child.once("exit", (code) => reject(new Error(`${what} exited with ${code}`)));
  });
}

/**
 * An address on this host that both the emulator (through its NAT) and this host reach, for the
 * ntfy server: the push endpoint it hands out must work from the home server and from the
 * emulator, and the emulator's 10.0.2.2 alias for the host's loopback means nothing to the host.
 * docker0's address is preferred, since it is not exposed to the network.
 */
export function hostAddress() {
  const nets = networkInterfaces();
  const v4 = (name) => (nets[name] ?? []).find((a) => a.family === "IPv4" && !a.internal)?.address;
  if (v4("docker0")) return v4("docker0");
  const all = Object.keys(nets).map(v4).filter(Boolean);
  // A private address (a Docker bridge first) rather than one on a VPN or the internet.
  const ranked = [/^172\.(1[6-9]|2\d|3[01])\./, /^10\./, /^192\.168\./, /./];
  for (const r of ranked) {
    const a = all.find((x) => r.test(x));
    if (a) return a;
  }
  throw new Error("no non-loopback IPv4 address on this host for the ntfy server");
}

/** A client for the home server's API, signed in with the vault password. */
export class Server {
  constructor(base, password) {
    this.base = base;
    this.password = password;
    this.cookie = "";
  }

  async login() {
    const res = await fetch(`${this.base}/api/login`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ password: this.password }),
    });
    if (!res.ok) throw new Error(`login: ${res.status} ${await res.text()}`);
    this.cookie = res.headers.get("set-cookie").split(";")[0];
  }

  async req(method, path, body) {
    const res = await fetch(`${this.base}${path}`, {
      method,
      headers: { cookie: this.cookie, ...(body === undefined ? {} : { "content-type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    let json;
    try {
      json = text ? JSON.parse(text) : null;
    } catch {
      json = text;
    }
    if (!res.ok) {
      const e = new Error(`${method} ${path}: ${res.status} ${text.slice(0, 300)}`);
      e.status = res.status;
      e.body = json;
      throw e;
    }
    return json;
  }

  get = (p) => this.req("GET", p);
  post = (p, b = {}) => this.req("POST", p, b);
  put = (p, b) => this.req("PUT", p, b);
  patch = (p, b) => this.req("PATCH", p, b);
  del = (p) => this.req("DELETE", p);

  /** Rows of query `q`. */
  async query(q) {
    const out = await this.post("/api/query", { query: q });
    return out.rows ?? out.items ?? out;
  }
}

// ---------------------------------------------------------------------------------------------
// Playwright, for the app's WebView

/** Playwright's experimental Android support, from the web client's dependencies. */
export function playwrightAndroid() {
  const require = createRequire(join(repo, "web", "package.json"));
  return require("@playwright/test")._android;
}

/** The app's WebView page, through adb (debuggable builds expose it). */
export async function appPage(timeout = 30_000) {
  const android = playwrightAndroid();
  const [device] = await android.devices();
  if (!device) throw new Error("Playwright sees no Android device");
  const webview = await device.webView({ pkg: PKG }, { timeout });
  const page = await webview.page();
  return { device, page };
}

// ---------------------------------------------------------------------------------------------
// Processes

/** Starts `cmd args`, logging to `log`, and answers the child. */
export function start(cmd, args, log, opts = {}) {
  const { openSync } = require_fs();
  const fd = openSync(log, "a");
  return spawn(cmd, args, { stdio: ["ignore", opts.pipe ? "pipe" : fd, fd], ...opts });
}

function require_fs() {
  return createRequire(import.meta.url)("node:fs");
}

/** Stops `child`, SIGKILL after 5 s. */
export function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    child.once("exit", () => resolve());
    child.kill("SIGTERM");
    setTimeout(() => child.kill("SIGKILL"), 5_000).unref();
  });
}

/** Downloads `url` to `path` unless it is already there. */
export function download(url, path) {
  if (existsSync(path)) return path;
  execFileSync("curl", ["-fsSL", "-o", path, url], { stdio: "inherit" });
  return path;
}

export const run = (cmd, args, opts = {}) =>
  new Promise((resolve, reject) =>
    execFile(cmd, args, { encoding: "utf8", maxBuffer: 64 << 20, ...opts }, (err, stdout, stderr) =>
      err ? reject(Object.assign(err, { stdout, stderr })) : resolve(stdout),
    ),
  );
