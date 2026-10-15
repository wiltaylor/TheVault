// Runs docs/android-manual-checks.md on the Android 15 emulator, as far as an emulator allows,
// and writes what happened to each check to <out>/RESULTS.md and <out>/results.json, with
// screenshots under <out>/shots and logs under <out>/logs.
//
//   ./dev just android-emulator-checks          (boots the emulator, builds what is missing)
//   node apps/android/emulator/checks.mjs       (inside ./dev, once the emulator is booted)
//
// Environment:
//   THEVAULT_APK            the APK to test (default: the x86_64 debug APK `just android-build x86_64` writes)
//   THEVAULT_EMULATOR_OUT   where results go (default .scratch/emulator)
//   THEVAULT_CHECKS         comma-separated check numbers to report; the setup the others need still runs
//   THEVAULT_PERIODIC_WAIT  minutes to wait for the 15-minute background work (default 17)
//
// What the emulator stands in for, and how:
//   - The home server is a vault-server on a fresh temporary vault, on 127.0.0.1, which the
//     emulator reaches as 10.0.2.2. "The web app" is its API: the web client shows the same data.
//   - UnifiedPush is a local ntfy server and the ntfy app, configured to it through its UI.
//   - Google Messages is removed from the system image and replaced by a test-only stand-in with
//     its package name (fake-messages/), which is the default SMS app: it stores each SMS the
//     emulator's modem delivers and posts Google-Messages-style notifications. It also inserts
//     incoming MMS into the Telephony provider and posts RCS notifications on command, and is the
//     app files are shared from.
//   - Calls come from the emulator console (gsm call/accept/cancel); contacts are edited through
//     the Contacts provider with `content`.
//   - Allow restricted settings: an adb install is exempt, so the appop Android 15 sets for an
//     app from an untrusted installer (ACCESS_RESTRICTED_SETTINGS=deny) is set by hand first.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as L from "./lib.mjs";

const { repo, PKG, MAIN, MESSAGES, NTFY, sleep, until, shell, shellTry, adb, adbTry, emu } = L;

const out = join(repo, process.env.THEVAULT_EMULATOR_OUT ?? ".scratch/emulator");
const shots = join(out, "shots");
const logs = join(out, "logs");
mkdirSync(shots, { recursive: true });
mkdirSync(logs, { recursive: true });
const cache = join(repo, "target/android-emulator");
mkdirSync(cache, { recursive: true });

const apk = process.env.THEVAULT_APK ?? join(repo, "apps/android/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");
const only = process.env.THEVAULT_CHECKS ? new Set(process.env.THEVAULT_CHECKS.split(",").map(Number)) : null;
const periodicWait = Number(process.env.THEVAULT_PERIODIC_WAIT ?? 17) * 60_000;

const NTFY_APK = "https://github.com/binwiederhier/ntfy-android/releases/download/v1.25.2/ntfy-1.25.2-fdroid-release.apk";
const NTFY_SERVER = "https://github.com/binwiederhier/ntfy/releases/download/v2.30.0/ntfy_2.30.0_linux_amd64.tar.gz";
const PASSWORD = "emulator-checks-password";
const DEVICE_NAME = "Emulator Pixel";

// Numbers the checks use: valid US numbers, since the emulator's SIM is American.
const KNOWN = "6502530001"; // a contact and a person
const KNOWN_E164 = "+16502530001";
const UNKNOWN = "6502539999";
const OUTGOING = "6502530003";
const OLD = "6502537777"; // before pairing: must never arrive
const SELF = "15555215554"; // the emulator's own number: an SMS sent to it comes back

// ---------------------------------------------------------------------------------------------
// Results

const TITLES = {};
for (const line of readFileSync(join(repo, "docs/android-manual-checks.md"), "utf8").split("\n")) {
  const m = /^\| (\d+) \| (.*?) \| (.*?) \|$/.exec(line);
  if (m) TITLES[m[1]] = { doing: m[2], expect: m[3] };
}

const results = {};
function record(n, status, note, evidence = []) {
  if (only && !only.has(n)) return;
  results[n] = { status, note, evidence: evidence.filter(Boolean).map((p) => p.replace(`${repo}/`, "")) };
  console.log(`[${String(n).padStart(2)}] ${status.toUpperCase()}: ${note}`);
  writeFileSync(join(out, "results.json"), JSON.stringify(results, null, 2));
}

/** Checks whose captured notes reached the server only after a nudge (see [captured]). */
const late = new Set();
let current = 0;

/**
 * A `type` note satisfying `pred` on the server: captured on the phone, it should arrive within
 * seconds. After 30 s the app is brought to the foreground, which runs a sync round, so the rest
 * of the check can still judge the note; the check then fails for the delay.
 */
async function captured(type, pred) {
  let found;
  const r = await openThenNudge(`the ${type} note on the server`, async () => (found = (await notesOf(type, pred))[0]), 30_000, 60_000);
  if (r.nudged) late.add(current);
  return found;
}

/** Like [captured], for any condition a phone-side change should bring about on the server. */
async function capturedUntil(what, fn) {
  let found;
  const r = await openThenNudge(what, async () => (found = await fn()), 30_000, 60_000);
  if (r.nudged) late.add(current);
  return found;
}

/** Runs check `n`: `body` answers [status, note, evidence] or throws, which is a fail. */
async function check(n, body) {
  const t0 = Date.now();
  current = n;
  try {
    let [status, note, evidence] = await body();
    if (late.has(n)) {
      note = `${note} BUT the note did not reach the server within 30 s; it arrived only after the app was brought to the foreground (a sync round). ${status === "pass" ? "Its content was right." : ""}`;
      status = "fail";
    }
    if (status === "fail") evidence = [...(evidence ?? []), saveLog(n)];
    record(n, status, note, evidence);
  } catch (e) {
    const shot = trySnap(`${n}-error`);
    record(n, "fail", `${e.message}`, [shot, saveLog(n)]);
  }
  console.log(`     (${((Date.now() - t0) / 1000).toFixed(1)} s)`);
}

const snap = (name) => L.screenshot(shots, name);
/** Saves the log so far for a failed check, and answers its path. */
function saveLog(n) {
  try {
    const path = join(logs, `${n}-logcat.txt`);
    L.logcatSave(path);
    // The app's capture state: its provider cursors and the device event queue (root shell).
    const data = `/data/user/0/${PKG}`;
    const state = shellTry(`cat ${data}/shared_prefs/thevault.device.cursors.xml; sqlite3 ${data}/vault/.vault/state.db "select id, kind, status, substr(payload, 1, 160), message from device_events" 2>&1 || sqlite3 ${data}/vault/.vault/state.db "select * from device_events" 2>&1`);
    writeFileSync(path.replace("-logcat.txt", "-capture-state.txt"), state);
    return path;
  } catch {
    return null;
  }
}
function trySnap(name) {
  try {
    return snap(name);
  } catch {
    return null;
  }
}
const elapsed = (t0) => `${((Date.now() - t0) / 1000).toFixed(1)} s`;

// ---------------------------------------------------------------------------------------------
// Environment

const work = mkdtempSync(join(tmpdir(), "thevault-emulator-"));
const children = [];
let srv;
let ntfyUrl;

async function startServer() {
  const vault = join(work, "vault");
  const env = { ...process.env, XDG_CONFIG_HOME: join(work, "config") };
  execFileSync(join(repo, "target/debug/vault"), ["init", vault, "--starter", "--password-stdin"], { input: PASSWORD, env, stdio: ["pipe", "ignore", "inherit"] });
  const child = L.start(join(repo, "target/debug/vault-server"), ["--vault", vault, "--web", join(repo, "web/dist"), "--listen", "127.0.0.1:0"], join(logs, "vault-server.log"), { pipe: true, env });
  children.push(child);
  const { listen } = JSON.parse(await L.firstLine(child, "vault-server"));
  const port = listen.split(":").pop();
  srv = new L.Server(`http://127.0.0.1:${port}`, PASSWORD);
  srv.devicePort = port;
  await srv.login();
  console.log(`home server on ${srv.base}, vault ${vault}`);
}

async function startNtfy() {
  const bin = join(cache, "ntfy");
  if (!existsSync(bin)) {
    execFileSync("bash", ["-c", `curl -fsSL ${NTFY_SERVER} | tar -xz -C ${cache} --strip-components=1 --wildcards '*/ntfy'`], { stdio: "inherit" });
  }
  const host = L.hostAddress();
  const port = await L.freePort(host);
  ntfyUrl = `http://${host}:${port}`;
  writeFileSync(join(work, "ntfy.yml"), "");
  const child = L.start(bin, ["serve", "--config", join(work, "ntfy.yml"), "--listen-http", `${host}:${port}`, "--base-url", ntfyUrl, "--cache-file", ""], join(logs, "ntfy.log"));
  children.push(child);
  await until("ntfy", async () => (await fetch(`${ntfyUrl}/v1/health`)).ok, { timeout: 20_000 });
  console.log(`ntfy on ${ntfyUrl}`);
}

/** Removes the real Google Messages from the system image, once per AVD, so the stand-in can take its name. */
async function replaceGoogleMessages() {
  root();
  const path = shellTry(`pm path ${MESSAGES}`);
  if (path.includes("/product/priv-app/PrebuiltBugle")) {
    console.log("removing the system image's Google Messages (needs -writable-system)");
    // remount may need a reboot (or two, after a wipe) before /product is writable.
    for (let i = 0; ; i++) {
      adbTry("remount");
      if (!shellTry("touch /product/priv-app/.thevault-rw && rm /product/priv-app/.thevault-rw && echo WRITABLE_OK").includes("WRITABLE_OK")) {
        if (i >= 3) throw new Error("could not remount /product writable; is the emulator running with -writable-system?");
        await reboot();
        root();
        continue;
      }
      break;
    }
    shell("rm -rf /product/priv-app/PrebuiltBugle");
    await reboot();
    root();
  } else if (path.includes("package:") && !path.includes("/data/app/")) {
    throw new Error(`unexpected ${MESSAGES}: ${path}`);
  }
  const helper = join(cache, "fake-messages.apk");
  execFileSync(join(repo, "apps/android/emulator/fake-messages/build.sh"), { stdio: "inherit" });
  adb("install", "-r", "-g", helper);
  shell(`cmd role add-role-holder android.app.role.SMS ${MESSAGES}`);
  const holder = shell("cmd role get-role-holders android.app.role.SMS").trim();
  if (holder !== MESSAGES) throw new Error(`the SMS role is held by "${holder}"`);
}

function root() {
  adbTry("root");
  adbTry("wait-for-device");
  for (let i = 0; i < 20 && !shellTry("id").includes("uid=0"); i++) execFileSync("sleep", ["0.5"]);
}

async function reboot() {
  adbTry("reboot");
  await sleep(5_000);
  adbTry("wait-for-device");
  await until("boot", () => shellTry("getprop sys.boot_completed").trim() === "1", { timeout: 300_000, every: 1_000 });
  shellTry("input keyevent KEYCODE_WAKEUP");
  shellTry("wm dismiss-keyguard");
  await sleep(3_000);
}

/** Installs ntfy and points its default server, which UnifiedPush registrations use, at ours. */
async function setUpNtfy() {
  const ntfyApk = L.download(NTFY_APK, join(cache, "ntfy.apk"));
  adbTry("uninstall", NTFY);
  adb("install", "-r", "-g", ntfyApk);
  shell(`monkey -p ${NTFY} -c android.intent.category.LAUNCHER 1`);
  await L.tapIf(/^allow$/i, 4_000);
  await L.tap("More options", { timeout: 15_000 });
  await L.tap(/^settings$/i);
  await L.tap(/^default server$/i);
  const field = await L.waitFor((n) => n.cls === "android.widget.EditText");
  await L.fill(field, ntfyUrl);
  await L.tap(/^save$/i);
  await L.waitFor(ntfyUrl);
  L.home();
  // ntfy's banner asks for battery optimisation off: otherwise Android 14 and later refuse to
  // start its subscriber service from the background when an app registers.
  shell(`dumpsys deviceidle whitelist +${NTFY}`);
}

function wakeAndUnlock() {
  shellTry("input keyevent KEYCODE_WAKEUP");
  shellTry("wm dismiss-keyguard");
}

const screenOff = () => shellTry("input keyevent KEYCODE_SLEEP");
const openApp = () => shell(`am start -n ${MAIN}`);
/** Leaves the app and kills its process, as Android does to a backgrounded app; jobs still run. */
function closeApp() {
  L.home();
  execFileSync("sleep", ["1"]);
  shellTry(`am kill ${PKG}`);
}
/**
 * Brings the app back to the foreground, which starts a sync round (docs/sync.md "When the Android
 * app syncs"): while it stays open, the app pulls server changes only on its own writes.
 */
async function foreground() {
  L.home();
  await sleep(1_000);
  openApp();
  await sleep(2_000);
}
/**
 * Waits `first` ms for `fn` with the app left open; if it has not happened, brings the app to the
 * foreground (a sync round) and waits `after` ms more. Answers { took, nudged }.
 */
async function openThenNudge(what, fn, first = 60_000, after = 60_000) {
  const t0 = Date.now();
  try {
    await until(what, fn, { timeout: first, every: 1_000 });
    return { took: elapsed(t0), nudged: false };
  } catch {
    await foreground();
    await until(what, fn, { timeout: after, every: 1_000 });
    return { took: elapsed(t0), nudged: true };
  }
}
const airplane = (on) => shell(`cmd connectivity airplane-mode ${on ? "enable" : "disable"}`);

/** The id of the device this run paired, from the home server. */
async function deviceRecord() {
  const { devices } = await srv.get("/api/devices");
  return devices.filter((d) => d.platform === "android").sort((a, b) => b.created - a.created)[0];
}

const rows = async (q) => (await srv.post("/api/query", { query: q })).rows;
const note = (id) => srv.get(`/api/notes/${id}`);
async function createNote(type, fields, blocks = []) {
  return srv.post(`/api/notes?type=${type}`, { note: { fields, blocks } });
}
async function updateNote(id, change) {
  const cur = await note(id);
  const next = structuredClone(cur.note);
  change(next);
  return srv.put(`/api/notes/${id}`, { note: next, base_version: cur.version });
}

/** The app's local API, through its WebView: what the phone's own vault holds. */
let pw;
async function page() {
  if (pw) {
    try {
      await pw.page.evaluate(() => 1);
      return pw.page;
    } catch {
      await pw.device.close().catch(() => {});
      pw = null;
    }
  }
  for (let attempt = 1; ; attempt++) {
    try {
      pw = await L.appPage(30_000);
      return pw.page;
    } catch (e) {
      if (attempt >= 3) throw e;
      // After a restart or a reinstall the WebView may not be up yet: bring the app to the front.
      wakeAndUnlock();
      openApp();
      await sleep(5_000);
    }
  }
}
async function local(method, path, body) {
  const p = await page();
  return p.evaluate(
    async ([method, path, body]) => {
      // As web/src/api.ts does: in the Android app a request with a body goes over Tauri IPC,
      // since the custom scheme drops request bodies (ticket 77).
      const ipc = globalThis.__thevaultApiOverIpc && globalThis.__TAURI_INTERNALS__;
      if (ipc && body) {
        const answer = await ipc.invoke("api_request", {
          request: { method, path, headers: [["Content-Type", "application/json"], ["Accept", "application/json"]], text: JSON.stringify(body) },
        });
        return { status: answer.status, body: typeof answer.body === "string" ? answer.body : new TextDecoder().decode(new Uint8Array(answer.body ?? [])) };
      }
      const res = await fetch(path, { method, headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
      return { status: res.status, body: await res.text() };
    },
    [method, path, body],
  );
}

/** Runs every WorkManager job of the app now, as if its constraints and delay were met. */
function runJobs() {
  const dumpOut = shellTry("dumpsys jobscheduler");
  const ids = [...new Set([...dumpOut.matchAll(new RegExp(`JOB androidx\\.work\\.systemjobscheduler:u0a\\d+/(\\d+): \\S+ @androidx\\.work\\.systemjobscheduler@${PKG.replace(/\./g, "\\.")}/`, "g"))].map((m) => m[1]))];
  for (const id of ids) shellTry(`cmd jobscheduler run -f -n androidx.work.systemjobscheduler ${PKG} ${id}`);
  return ids;
}

const smsInbox = () => shellTry("content query --uri content://sms/inbox --projection _id:address:body:date");

// Contacts, through the provider.
/** Adds a contact in one batch, as the Contacts app does, through the stand-in; answers its raw contact id. */
function addContact(name, number) {
  const out = shell(`am broadcast -n ${MESSAGES}/.CommandReceiver -a thevault.test.CONTACT --es name '${name}' --es phone '${number}'`);
  const id = /data="(\d+)"/.exec(out)?.[1];
  if (!id) throw new Error(`adding contact ${name}: ${out}`);
  return id;
}
function addPhone(rawId, number) {
  shell(`content insert --uri content://com.android.contacts/data --bind raw_contact_id:i:${rawId} --bind mimetype:s:vnd.android.cursor.item/phone_v2 --bind data1:s:'${number}' --bind data2:i:2`);
}
function setPhone(rawId, number) {
  shell(`content update --uri content://com.android.contacts/data --bind data1:s:'${number}' --where "raw_contact_id=${rawId} AND mimetype='vnd.android.cursor.item/phone_v2'"`);
}
function setName(rawId, name) {
  shell(`content update --uri content://com.android.contacts/data --bind data1:s:'${name}' --where "raw_contact_id=${rawId} AND mimetype='vnd.android.cursor.item/name'"`);
}
function contactData(where) {
  return shellTry(`content query --uri content://com.android.contacts/data --projection raw_contact_id:mimetype:data1 --where "${where}"`);
}
function phoneContacts() {
  return shellTry(`content query --uri content://com.android.contacts/raw_contacts --projection _id:display_name:deleted --where "deleted=0"`);
}

/** The person notes, with their fields. */
async function people() {
  const list = await rows("from person");
  return Promise.all(list.map(async (r) => ({ id: r.id, ...(await note(r.id)) })));
}
const phonesOf = (p) => p.note.fields.phones ?? [];
const linksOf = (p) => p.note.blocks.find((b) => b.kind === "dict" && b.labels?.[0] === "device_contacts")?.fields ?? p.note.fields.device_contacts ?? {};

/** Message or call notes whose fields satisfy `pred`, with the full note. */
async function notesOf(type, pred = () => true) {
  const list = await rows(`from ${type}`);
  const full = await Promise.all(list.map(async (r) => ({ id: r.id, ...(await note(r.id)) })));
  return full.filter((n) => pred(n.note.fields, n));
}

/** Plain text of a note's content blocks. */
function textOf(n) {
  const parts = [];
  const walk = (b) => {
    if (!b) return;
    if (Array.isArray(b)) return b.forEach(walk);
    if (typeof b === "string") return parts.push(b);
    for (const v of Object.values(b.fields ?? {})) if (typeof v === "string") parts.push(v);
    if (b.text) parts.push(b.text);
    walk(b.blocks);
    walk(b.content);
    walk(b.children);
  };
  walk(n.note.blocks);
  for (const v of Object.values(n.note.fields)) if (typeof v === "string") parts.push(v);
  return parts.join("\n");
}

/** The server's notification ids that are unread. */
async function unread() {
  const r = await srv.get("/api/notifications");
  return (r.items ?? r.notifications ?? r).filter((n) => !n.read);
}
const shown = (title) => L.appNotifications().find((n) => n.title === title);

/** Waits for the Android notification titled `title` to show (or, with `gone`, to go), answering the seconds it took. */
async function timeNotification(title, { gone = false, timeout = 20_000 } = {}) {
  const t0 = Date.now();
  await until(`notification "${title}" to ${gone ? "go" : "show"}`, () => (gone ? !shown(title) : shown(title)), { timeout, every: 250 });
  return (Date.now() - t0) / 1000;
}

/** Queues device command `name` with `payload` for the paired device, through a scheduled script run. */
async function queueCommand(name, payload) {
  const dev = await deviceRecord();
  const source = `use vault\n\nfn run() -> Result[(), string] {\n    let payload = Value::Map(#{\n${Object.entries(payload)
    .map(([k, v]) => `        "${k}": Value::String("${v}"),`)
    .join("\n")}\n    })\n    vault::device_command("${dev.id}", "${name}", payload)?\n    Ok(())\n}\n`;
  const task = `emu_${name}`;
  await srv.put(`/api/scripts/${task}`, { source });
  await srv.put(`/api/tasks/${task}`, { wcl: `schedule = "0 0 1 1 *"\nscript = "${task}"\nretries = 0\n` });
  const run = await srv.post(`/api/tasks/${task}/run`, {});
  const { commands } = await srv.get(`/api/devices/${dev.id}/commands`);
  const cmd = commands.filter((c) => c.name === name).sort((a, b) => b.created - a.created)[0];
  if (!cmd) throw new Error(`no ${name} command queued: run ${JSON.stringify(run).slice(0, 400)}`);
  return { dev, cmd };
}
async function commandStatus(devId, id) {
  const { commands } = await srv.get(`/api/devices/${devId}/commands`);
  return commands.find((c) => c.id === id);
}

async function setDevice(settings) {
  const dev = await deviceRecord();
  return srv.patch(`/api/devices/${dev.id}`, { settings });
}

// ---------------------------------------------------------------------------------------------
// The onboarding flow, which several checks run

const edits = () => L.dump().filter((n) => n.cls === "android.widget.EditText");

async function pairWith(password, name = DEVICE_NAME) {
  let e = await until("the onboarding fields", () => {
    const x = edits();
    return x.length >= 4 && x;
  });
  await L.fill(e[0], `http://10.0.2.2:${srv.devicePort}`);
  e = edits();
  await L.fill(e[1], password);
  e = edits();
  await L.fill(e[2], name);
  await L.tap(/^pair$/i);
}

// ---------------------------------------------------------------------------------------------
// The checks

async function main() {
  console.log(`results in ${out}`);
  if (!existsSync(apk)) throw new Error(`${apk} is missing; run \`just android-build x86_64\` or set THEVAULT_APK`);
  const device = {
    release: shell("getprop ro.build.version.release").trim(),
    sdk: shell("getprop ro.build.version.sdk").trim(),
    model: shell("getprop ro.product.model").trim(),
    image: shell("getprop ro.build.fingerprint").trim(),
  };
  console.log(`device: ${JSON.stringify(device)}`);

  await replaceGoogleMessages();
  await startServer();
  await startNtfy();
  wakeAndUnlock();
  await setUpNtfy();
  shellTry("settings put global auto_time 1");

  adbTry("uninstall", PKG);
  const installed = adb("install", apk);
  console.log(`installed ${apk}: ${installed.trim().split("\n").pop()}`);
  writeFileSync(join(out, "run.json"), JSON.stringify({ started: new Date().toISOString(), apk, device, server: srv.base, ntfy: ntfyUrl }, null, 2));

  // Things on the phone before pairing, which the import (left off) must not bring in (check 28).
  emu(`sms send ${OLD} Old message from before pairing`);
  shell(`content insert --uri content://call_log/calls --bind number:s:${OLD} --bind type:i:1 --bind duration:i:42 --bind date:l:${Date.now() - 3_600_000} --bind new:i:0`);
  const oldContact = addContact("Old Contact", "+16502537777");
  await sleep(2_000);

  L.logcatClear();

  // 1. First launch.
  await check(1, async () => {
    openApp();
    await L.waitFor("Connect TheVault to your server", { timeout: 30_000 });
    return ["pass", "The onboarding screen opened over the app.", [snap("01-onboarding")]];
  });
  if (results[1]?.status === "fail" || (!results[1] && !L.find("Connect TheVault to your server"))) {
    const lines = L.logcatSave(join(logs, "launch-logcat.txt"), /RustStdoutStderr|FATAL|AndroidRuntime|Fatal signal/);
    writeFileSync(join(logs, "launch-crash-excerpt.txt"), lines.join("\n"));
    throw new Error(`the app did not start; see ${join(logs, "launch-crash-excerpt.txt")}:\n${lines.slice(0, 6).join("\n")}`);
  }

  // 2. Wrong password.
  await check(2, async () => {
    await pairWith("not-the-password");
    await L.waitFor("wrong password", { timeout: 15_000 });
    const still = L.find("Connect TheVault to your server");
    return [still ? "pass" : "fail", still ? `"wrong password" shown; the screen stayed open.` : "the screen closed", [snap("02-wrong-password")]];
  });

  // 3. Right password; allow notifications.
  await check(3, async () => {
    await pairWith(PASSWORD);
    await L.waitFor(/^allow$/i, { timeout: 30_000 });
    const prompt = snap("03-notification-permission");
    await L.tap(/^allow$/i);
    const dev = await until("the device on the server", deviceRecord);
    const ok = dev.name === DEVICE_NAME && dev.platform === "android";
    return [ok ? "pass" : "fail", `Paired. GET /api/devices lists "${dev.name}", platform ${dev.platform}, id ${dev.id}.`, [prompt]];
  });

  // 24 to 28: capture setup, right after pairing.
  shell(`appops set ${PKG} ACCESS_RESTRICTED_SETTINGS deny`);
  await check(24, async () => {
    await L.waitFor("Messages", { timeout: 20_000 });
    await L.tap(/^allow$/i);
    const denied = await L.waitFor(/denied access to SMS|restricted/i, { timeout: 10_000 });
    const dialog = snap("24-restricted-dialog");
    await L.tap(/^ok$/i);
    await L.waitFor(/Allow restricted settings/, { timeout: 10_000 });
    const info = L.find(/^open app info$/i);
    const shot = snap("24-restricted-steps");
    return [info ? "pass" : "fail", `Android answered "${denied.text}" instead of the permission dialog; the app showed the three Allow restricted settings steps and Open App info. (Restricted state simulated: appops ACCESS_RESTRICTED_SETTINGS deny, since adb installs are exempt.)`, [dialog, shot]];
  });

  await check(25, async () => {
    await L.tap(/^open app info$/i);
    await L.tap("More options", { timeout: 10_000 });
    await L.tap(/^allow restricted settings$/i);
    await sleep(1_000);
    const mode = shell(`appops get ${PKG} ACCESS_RESTRICTED_SETTINGS`).trim();
    L.back();
    await L.tap(/^try again$/i, { timeout: 10_000 });
    await L.waitFor(/send and view SMS/i, { timeout: 10_000 });
    const ask = snap("25-sms-permission");
    await L.tap(/^allow$/i);
    const next = await L.waitFor((n) => n.pkg === PKG && /^(RCS messages|Calls)$/.test(n.text), { timeout: 10_000 });
    const shot = snap("25-next-step");
    const note = `Allow restricted settings set the appop to "${mode}"; Try again brought the SMS permission dialog; allowing it moved on to "${next.text}".` +
      (next.text === "RCS messages" ? " The checklist says the Calls step comes next, but the app puts the RCS messages step (notification access, check 27) between them: the checklist is out of date, not the app." : "");
    return ["pass", note, [ask, shot]];
  });

  await check(27, async () => {
    await L.waitFor("RCS messages");
    await L.tap(/^open notification access$/i);
    await L.tap(/^allow notification access$/i, { timeout: 10_000 });
    await L.tap(/^allow$/i, { timeout: 10_000 });
    await sleep(1_000);
    const listeners = shell("settings get secure enabled_notification_listeners");
    const shot = snap("27-notification-access");
    L.back();
    const on = listeners.includes(PKG);
    return [on ? "pass" : "fail", `Notification access for TheVault RCS capture is ${on ? "on" : "off"} (enabled_notification_listeners: ${listeners.trim().slice(0, 200)}). It was not greyed out, since restricted settings were already allowed in check 25.`, [shot]];
  });

  await check(26, async () => {
    const asked = [];
    await L.waitFor("Calls", { timeout: 10_000 });
    await L.tap(/^allow$/i);
    for (let i = 0; i < 4; i++) {
      const d = L.find((n) => n.pkg.includes("permissioncontroller") && /^Allow TheVault/.test(n.text));
      if (!d) break;
      asked.push(d.text);
      await L.tap(/^allow$/i);
      await sleep(800);
    }
    await L.waitFor("Contacts", { timeout: 10_000 });
    await L.tap(/^allow$/i);
    for (let i = 0; i < 3; i++) {
      const d = L.find((n) => n.pkg.includes("permissioncontroller") && /^Allow TheVault/.test(n.text));
      if (!d) break;
      asked.push(d.text);
      await L.tap(/^allow$/i);
      await sleep(800);
    }
    await L.waitFor("Sending SMS", { timeout: 10_000 });
    await L.tap(/^skip$/i);
    let notif = "not shown: POST_NOTIFICATIONS was granted at the end of onboarding";
    if (L.find("Notifications")) {
      await L.tap(/^allow$/i);
      await L.tapIf(/^allow$/i, 3_000);
      notif = "asked and allowed";
    }
    const page = await L.waitFor("Earlier messages", { timeout: 10_000 });
    const shot = snap("26-import-page");
    const sendSms = shell(`dumpsys package ${PKG}`).includes("android.permission.SEND_SMS: granted=true");
    return [page && !sendSms ? "pass" : "fail", `Calls and Contacts each asked once: ${asked.join(" / ")}. Notifications step: ${notif}. Skipping Sending SMS moved on without asking (SEND_SMS granted: ${sendSms}).`, [shot]];
  });

  await check(28, async () => {
    const sw = L.find((n) => n.cls === "android.widget.Switch");
    if (sw?.checked) throw new Error("the import switch is on by default");
    await L.tap(/^finish$/i);
    await sleep(3_000);
    const p = await page();
    const url = p.url();
    const shot = snap("28-app-open");
    await sleep(20_000);
    const msgs = await rows("from message");
    const calls = await rows("from call");
    const old = (await people()).filter((x) => x.note.fields.title === "Old Contact");
    const ok = msgs.length === 0 && calls.length === 0 && old.length === 0;
    return [ok ? "pass" : "fail", `The app opened at ${url}. After 20 s the server holds ${msgs.length} message, ${calls.length} call and ${old.length} "Old Contact" person notes (the SMS, call and contact made before pairing).`, [shot]];
  });

  // 6. Close and reopen.
  await check(6, async () => {
    shell(`am force-stop ${PKG}`);
    openApp();
    await sleep(6_000);
    const onboarding = L.find("Connect TheVault to your server") || L.find("Vault password");
    const shot = snap("06-reopened");
    const p = await page();
    return [onboarding ? "fail" : "pass", onboarding ? "the onboarding screen came back" : `No onboarding screen; the app opened at ${p.url()}.`, [shot]];
  });

  // 16. Push on.
  await check(16, async () => {
    const dev = await until("push on for the device", async () => {
      const d = await deviceRecord();
      return d.push && d;
    }, { timeout: 30_000 });
    const chooser = L.find(/ntfy/i);
    return ["pass", `One distributor (ntfy) is installed, so Android asked nothing${chooser ? " (a chooser was on screen)" : ""}. GET /api/devices shows push: ${dev.push} for ${dev.name}; ntfy endpoint on ${ntfyUrl}.`, [snap("16-after-open")]];
  });

  // 7. A note created on the server reaches the app.
  await check(7, async () => {
    const created = await createNote("note", { title: "Made in the web app" });
    const t0 = Date.now();
    openApp();
    await until("the note in the app's vault", async () => (await local("GET", `/api/notes/${created.id}`)).status === 200, { timeout: 60_000, every: 500 });
    const took = elapsed(t0);
    const p = await page();
    await p.goto(`http://vault.localhost/notes/${created.id}`);
    await p.getByRole("heading", { name: "Made in the web app" }).first().waitFor({ timeout: 15_000 }).catch(() => p.getByText("Made in the web app").first().waitFor({ timeout: 15_000 }));
    await sleep(1_500);
    return ["pass", `The note was in the app's vault ${took} after the app opened, and the app shows it.`, [snap("07-note-in-app")]];
  });

  // 8. An edit in the app reaches the server.
  await check(8, async () => {
    const created = await createNote("note", { title: "Edit me on the phone" });
    await foreground();
    await until("the note in the app", async () => (await local("GET", `/api/notes/${created.id}`)).status === 200, { timeout: 60_000 });
    const p = await page();
    await p.goto(`http://vault.localhost/notes/${created.id}`);
    const title = p.getByLabel("Title", { exact: true });
    await title.waitFor({ timeout: 15_000 });
    await title.fill("Edited on the phone");
    await p.keyboard.press("Tab");
    await p.getByTestId("save-status").filter({ hasText: "Saved" }).waitFor({ timeout: 15_000 }).catch(() => {});
    const t0 = Date.now();
    try {
      await until("the edit on the server", async () => (await note(created.id)).note.fields.title === "Edited on the phone", { timeout: 60_000 });
    } catch (e) {
      const shot = snap("08-edit-failed");
      const errors = await p.locator("text=/not JSON|error|failed/i").allTextContents().catch(() => []);
      const probe = await local("POST", "/api/notes?type=note", { note: { fields: { title: "probe" }, blocks: [] } });
      return ["fail", `Edited the title in the app's editor; it never reached the server (60 s). The app shows: ${JSON.stringify(errors.slice(0, 3))}. A POST from the WebView to its own /api answers ${probe.status} ${probe.body.slice(0, 160)}: the request body does not reach the embedded server.`, [shot]];
    }
    return ["pass", `Edited the title in the app's editor; the server had it ${elapsed(t0)} after the save.`, [snap("08-edited")]];
  });

  // 10. A file the phone never showed.
  await check(10, async () => {
    const png = execFileSync("qrencode", ["-o", "-", "-s", "8", "TheVault check 10"]);
    const res = await fetch(`${srv.base}/api/assets?filename=check10.png`, { method: "POST", headers: { cookie: srv.cookie, "content-type": "image/png" }, body: png });
    if (!res.ok) throw new Error(`upload: ${res.status} ${await res.text()}`);
    const asset = (await res.json()).id;
    const created = await createNote("note", { title: "Note with an image" }, [{ kind: "content", labels: ["body"], blocks: [{ kind: "image", fields: { asset, alt: "check 10" } }] }]);
    await foreground();
    await until("the note in the app", async () => (await local("GET", `/api/notes/${created.id}`)).status === 200, { timeout: 60_000 });
    const before = await local("HEAD", `/api/assets/${asset}`);
    const p = await page();
    await p.goto(`http://vault.localhost/notes/${created.id}`);
    const img = p.locator("figure.block-image img").first();
    await img.waitFor({ timeout: 20_000 });
    await until("the image to load", () => img.evaluate((el) => el.complete && el.naturalWidth > 0), { timeout: 30_000 });
    const width = await img.evaluate((el) => el.naturalWidth);
    return ["pass", `The image (asset ${asset}, not yet on the phone) loaded in the app, ${width} px wide, fetched from the server.`, [snap("10-image")]];
  });

  // 11 to 14: the share target. The stand-in app shares, as Chrome, Photos or Files would.
  const share = async (extras) => {
    const args = Object.entries(extras).map(([k, v]) => `--es ${k} '${v}'`).join(" ");
    shell(`am start -n ${MESSAGES}/.ShareSourceActivity ${args}`);
    const toast = await until("a toast", () => /TextToastRecord\{[^}]*text=([^}]*?) duration/.exec(shellTry("dumpsys notification"))?.[1], { timeout: 20_000, every: 150 });
    const shot = snap(`share-${Date.now()}`);
    return { toast, shot };
  };
  const fleeting = async (pred, timeout = 60_000) =>
    until("the fleeting note on the server", async () => (await notesOf("fleeting", pred))[0], { timeout });

  await check(11, async () => {
    wakeAndUnlock();
    const { toast, shot } = await share({ text: "Shared text for check eleven" });
    const n = await fleeting((f, x) => textOf(x).includes("Shared text for check eleven"));
    return [toast === "Saved to TheVault inbox" ? "pass" : "fail", `Toast "${toast}". Fleeting note ${n.id} "${n.note.fields.title}" is on the server.`, [shot]];
  });

  await check(12, async () => {
    const { toast, shot } = await share({ text: "https://example.com/check-twelve", subject: "Check Twelve Page Title" });
    const n = await fleeting((f, x) => textOf(x).includes("https://example.com/check-twelve"));
    const ok = toast === "Saved to TheVault inbox" && n.note.fields.title === "Check Twelve Page Title";
    return [ok ? "pass" : "fail", `Shared a link with its page title, as Chrome does (EXTRA_TEXT the URL, EXTRA_SUBJECT the title). Toast "${toast}". Note title "${n.note.fields.title}"; the body holds the link.`, [shot]];
  });

  await check(13, async () => {
    const { toast, shot } = await share({ files: "check13.png" });
    const n = await fleeting((f) => f.title === "check13.png" || JSON.stringify(f).includes("check13"));
    const images = n.note.fields.image ?? [];
    const p = await page();
    await p.goto(`http://vault.localhost/notes/${n.id}`);
    await sleep(3_000);
    const loaded = await p.locator("img").evaluateAll((els) => els.filter((e) => e.complete && e.naturalWidth > 0).length);
    const app = snap("13-image-note");
    return [toast === "Saved to TheVault inbox" && images.length === 1 && loaded > 0 ? "pass" : "fail", `Toast "${toast}". Note ${n.id} has ${images.length} image(s); the app shows ${loaded} loaded image(s).`, [shot, app]];
  });

  await check(14, async () => {
    const { toast, shot } = await share({ files: "check14-a.png,check14-b.png,check14.pdf" });
    const n = await fleeting((f, x) => JSON.stringify(x.note).includes("check14"));
    const json = JSON.stringify(n.note);
    const images = (n.note.fields.image ?? []).length;
    const assetIds = [...json.matchAll(/"asset":"([0-9A-Z]{26})"/g)].map((m) => m[1]);
    const listed = await srv.get("/api/assets");
    const pdf = (listed.assets ?? listed.items ?? listed).some((a) => assetIds.includes(a.id) && a.filename === "check14.pdf");
    const all = await notesOf("fleeting", (f, x) => JSON.stringify(x.note).includes("check14"));
    return [toast === "Saved to TheVault inbox" && images === 2 && pdf && all.length === 1 ? "pass" : "fail", `SEND_MULTIPLE of two images and a PDF. Toast "${toast}". ${all.length} note(s); ${images} image(s), PDF attached: ${pdf}.`, [shot]];
  });

  // 17 to 21: notifications through UnifiedPush.
  await check(17, async () => {
    closeApp();
    screenOff();
    await sleep(2_000);
    const n = await srv.post("/api/notifications", { title: "Check 17 title", body: "Check 17 body" });
    const took = await timeNotification("Check 17 title", { timeout: 30_000 });
    const a = shown("Check 17 title");
    wakeAndUnlock();
    shell("cmd statusbar expand-notifications");
    await sleep(1_000);
    const shot = snap("17-shade");
    shell("cmd statusbar collapse");
    const ok = took <= 10 && a.text === "Check 17 body";
    return [ok ? "pass" : "fail", `App closed, screen off. The notification showed ${took.toFixed(1)} s after it was created, title "${a.title}", body "${a.text}". (id ${n.id})`, [shot]];
  });

  await check(18, async () => {
    const [n] = (await unread()).filter((x) => x.title === "Check 17 title");
    if (!n || !shown("Check 17 title")) throw new Error("check 17's notification is not showing, so there is nothing to clear");
    closeApp();
    screenOff();
    const t0 = Date.now();
    await srv.post(`/api/notifications/${n.id}/read`);
    await timeNotification("Check 17 title", { gone: true, timeout: 30_000 });
    const took = (Date.now() - t0) / 1000;
    return [took <= 10 ? "pass" : "fail", `Marked read on the server; the Android notification went ${took.toFixed(1)} s later.`, []];
  });

  await check(19, async () => {
    const a = await srv.post("/api/notifications", { title: "Check 19 A" });
    const b = await srv.post("/api/notifications", { title: "Check 19 B" });
    await timeNotification("Check 19 A", { timeout: 30_000 });
    await timeNotification("Check 19 B", { timeout: 30_000 });
    let t0 = Date.now();
    await srv.del(`/api/notifications/${a.id}`);
    await timeNotification("Check 19 A", { gone: true, timeout: 30_000 });
    const dismissed = (Date.now() - t0) / 1000;
    t0 = Date.now();
    await srv.post(`/api/notifications/${b.id}/read`);
    await srv.del("/api/notifications");
    await timeNotification("Check 19 B", { gone: true, timeout: 30_000 });
    const cleared = (Date.now() - t0) / 1000;
    return [dismissed <= 10 && cleared <= 10 ? "pass" : "fail", `Dismissed A: gone ${dismissed.toFixed(1)} s later. Read B and cleared read notifications: gone ${cleared.toFixed(1)} s later.`, []];
  });

  async function swipeAway(title) {
    wakeAndUnlock();
    shell("cmd statusbar expand-notifications");
    const row = await L.waitFor(title, { timeout: 10_000 });
    shell(`input swipe ${row.x - 300} ${row.y} ${row.x + 700} ${row.y} 250`);
    await sleep(1_000);
    shell("cmd statusbar collapse");
  }

  await check(20, async () => {
    const n = await srv.post("/api/notifications", { title: "Check 20 swipe" });
    await timeNotification("Check 20 swipe", { timeout: 30_000 });
    await swipeAway("Check 20 swipe");
    if (shown("Check 20 swipe")) throw new Error("the swipe did not remove the notification");
    const t0 = Date.now();
    await until("read on the server", async () => !(await unread()).some((x) => x.id === n.id), { timeout: 30_000 });
    const online = (Date.now() - t0) / 1000;

    const m = await srv.post("/api/notifications", { title: "Check 20 offline swipe" });
    await timeNotification("Check 20 offline swipe", { timeout: 30_000 });
    airplane(true);
    await sleep(2_000);
    await swipeAway("Check 20 offline swipe");
    await sleep(10_000);
    const stillUnread = (await unread()).some((x) => x.id === m.id);
    airplane(false);
    const t1 = Date.now();
    await until("the offline read on the server", async () => !(await unread()).some((x) => x.id === m.id), { timeout: 120_000 });
    const later = (Date.now() - t1) / 1000;
    const ok = online <= 5 && stillUnread;
    return [ok ? "pass" : "fail", `Swiped: read on the server ${online.toFixed(1)} s later. Swiped in airplane mode: still unread after 10 s offline (${stillUnread}); read ${later.toFixed(1)} s after airplane mode went off.`, []];
  });

  await check(21, async () => {
    const target = await createNote("note", { title: "Opened from a notification" });
    await sleep(3_000);
    closeApp();
    const tapOpen = async (title) => {
      const n = await srv.post("/api/notifications", { title, note: target.id });
      await timeNotification(title, { timeout: 30_000 });
      wakeAndUnlock();
      shell("cmd statusbar expand-notifications");
      await L.tap(title, { timeout: 10_000 });
      await sleep(4_000);
      const p = await page();
      await until("the note page", () => p.url().includes(target.id), { timeout: 20_000 });
      await until("read on the server", async () => !(await unread()).some((x) => x.id === n.id), { timeout: 20_000 });
      return p.url();
    };
    const cold = await tapOpen("Check 21 cold");
    const shot = snap("21-opened-note");
    L.home();
    await sleep(1_500);
    const warm = await tapOpen("Check 21 warm");
    return ["pass", `Tapped with the app closed: opened ${cold}, marked read. Tapped with the app in the background: opened ${warm}, marked read.`, [shot]];
  });

  // 29 to 33: messages and calls, with the app in the foreground (sync loop running).
  openApp();
  await sleep(3_000);
  const knownPerson = await createNote("person", { title: "Kim Known", phones: [KNOWN_E164] });
  await foreground();
  await until("the person on the phone", async () => (await local("GET", `/api/notes/${knownPerson.id}`)).status === 200, { timeout: 60_000 });
  const knownContact = addContact("Kim Known", KNOWN_E164);
  await until("the contact linked to the person", async () => Object.keys(linksOf(await note(knownPerson.id))).length > 0, { timeout: 60_000 }).catch(() => {});

  const message = (pred) => captured("message", pred);

  await check(29, async () => {
    const t0 = Date.now();
    emu(`sms send ${KNOWN} Hello from a known contact`);
    const m = await message((f) => f.body === "Hello from a known contact" || JSON.stringify(f).includes("Hello from a known contact"));
    const took = elapsed(t0);
    const f = m.note.fields;
    const conv = f.conversation ? await note(f.conversation).catch(() => null) : null;
    const ok = f.from === knownPerson.id && conv?.type === "conversation";
    return [ok ? "pass" : "fail", `Message note ${m.id} on the server ${took} after the SMS: from=${f.from} (person ${knownPerson.id}), from_raw=${f.from_raw ?? ""}, conversation=${f.conversation} (${conv?.type ?? "missing"}), channel=${f.channel}.`, []];
  });

  await check(30, async () => {
    const before = (await people()).length;
    emu(`sms send ${UNKNOWN} Hello from a stranger`);
    const m = await message((f) => JSON.stringify(f).includes("Hello from a stranger"));
    await sleep(3_000);
    const after = (await people()).length;
    const f = m.note.fields;
    const ok = !f.from && String(f.from_raw ?? "").includes("6502539999") && after === before;
    return [ok ? "pass" : "fail", `Message note ${m.id}: from=${f.from ?? "(none)"}, from_raw=${f.from_raw}. People before ${before}, after ${after}.`, []];
  });

  await check(31, async () => {
    shell(`am broadcast -n ${MESSAGES}/.CommandReceiver -a thevault.test.MMS --es from ${KNOWN} --es text 'Picture message for check 31' --ez image true`);
    const m = await message((f) => JSON.stringify(f).includes("Picture message for check 31"));
    const f = m.note.fields;
    const atts = f.attachments ?? [];
    const p = await page();
    await until("the message in the app", async () => (await local("GET", `/api/notes/${m.id}`)).status === 200, { timeout: 60_000 });
    await p.goto(`http://vault.localhost/notes/${m.id}`);
    await sleep(4_000);
    const loaded = await p.locator("img").evaluateAll((els) => els.filter((e) => e.complete && e.naturalWidth > 0).length);
    const shot = snap("31-mms-note");
    const ok = atts.length === 1 && f.conversation && loaded > 0;
    return [ok ? "pass" : "fail", `MMS inserted into the Telephony provider by the stand-in (the emulator modem cannot deliver MMS). Message note ${m.id}: ${atts.length} attachment(s) ${JSON.stringify(atts).slice(0, 160)}, conversation=${f.conversation}, from=${f.from}; the app shows ${loaded} loaded image(s).`, [shot]];
  });

  await check(32, async () => {
    const rcsPerson = await createNote("person", { title: "Riley Rcs" });
    await foreground();
    await until("the person on the phone", async () => (await local("GET", `/api/notes/${rcsPerson.id}`)).status === 200, { timeout: 60_000 });
    L.home();
    shell(`am broadcast -n ${MESSAGES}/.CommandReceiver -a thevault.test.RCS --es conversation rcs-chat-1 --es sender 'Riley Rcs' --es text 'RCS hello for check 32'`);
    const m = await message((f) => JSON.stringify(f).includes("RCS hello for check 32"));
    const f = m.note.fields;
    // A group chat: two senders, one conversation; reposting it must not capture its messages twice.
    // Google Messages reposts a conversation with the same messages and times: the stand-in gets the time.
    const groupTime = Date.now();
    shell(`am broadcast -n ${MESSAGES}/.CommandReceiver -a thevault.test.RCS --es conversation rcs-group-1 --es group 'Weekend plans' --es sender 'Riley Rcs' --es text 'Group line one' --es sender2 'Sam Second' --es text2 'Group line two' --el time ${groupTime}`);
    const g1 = await message((x) => JSON.stringify(x).includes("Group line one"));
    const g2 = await message((x) => JSON.stringify(x).includes("Group line two"));
    shell(`am broadcast -n ${MESSAGES}/.CommandReceiver -a thevault.test.RCS --es conversation rcs-group-1 --es group 'Weekend plans' --es sender 'Riley Rcs' --es text 'Group line one' --es sender2 'Sam Second' --es text2 'Group line two' --el time ${groupTime}`);
    // An RCS-looking notification with the text of an SMS already in the provider: dropped.
    emu(`sms send ${UNKNOWN} Duplicate check text 32`);
    await message((x) => JSON.stringify(x).includes("Duplicate check text 32"));
    await sleep(15_000);
    const dupes = await notesOf("message", (x) => JSON.stringify(x).includes("Duplicate check text 32"));
    const groupOnes = await notesOf("message", (x) => JSON.stringify(x).includes("Group line one"));
    const smsNotified = (await notesOf("message", (x) => JSON.stringify(x).includes("Hello from a known contact"))).length;
    const oneConversation = g1.note.fields.conversation && g1.note.fields.conversation === g2.note.fields.conversation;
    const ok = f.from === rcsPerson.id && f.partial === true && oneConversation && dupes.length === 1 && groupOnes.length === 1 && smsNotified === 1;
    return [ok ? "pass" : "fail", `Posted by the test-only Google Messages stand-in (no carrier RCS in the emulator). 1:1 message ${m.id}: from=${f.from} (Riley Rcs ${rcsPerson.id}), from_raw=${f.from_raw ?? ""}, partial=${f.partial}, channel=${f.channel}. Group: two messages in conversation ${g1.note.fields.conversation} / ${g2.note.fields.conversation}; a repost captured ${groupOnes.length} copy of line one. An SMS and the notification the stand-in posts for it: ${dupes.length} message note(s); the earlier SMS of check 29: ${smsNotified}.`, []];
  });

  const call = (pred) => captured("call", pred);
  await check(33, async () => {
    wakeAndUnlock();
    emu(`gsm call ${KNOWN}`);
    await sleep(4_000);
    emu(`gsm accept ${KNOWN}`);
    await sleep(5_000);
    emu(`gsm cancel ${KNOWN}`);
    const inCall = await call((f) => f.direction === "in");
    await sleep(3_000);
    emu(`gsm call ${KNOWN}`);
    await sleep(5_000);
    emu(`gsm cancel ${KNOWN}`);
    const missed = await call((f) => f.direction === "missed");
    await sleep(3_000);
    shell(`am start -a android.intent.action.CALL -d tel:${OUTGOING}`);
    await sleep(6_000);
    const outShot = snap("33-outgoing-call");
    emu(`gsm accept ${OUTGOING}`);
    await sleep(4_000);
    // Hang up on the phone: the console's gsm cancel does not end an outgoing call.
    shell("input keyevent KEYCODE_ENDCALL");
    await sleep(1_000);
    emu(`gsm cancel ${OUTGOING}`);
    const outCall = await call((f) => f.direction === "out").catch((e) => null);
    L.home();
    const desc = (c) => (c ? `${c.id} direction=${c.note.fields.direction} person=${c.note.fields.person ?? "(none)"} number=${c.note.fields.number} duration=${c.note.fields.duration}` : "(none)");
    const ok = inCall.note.fields.person === knownPerson.id && missed.note.fields.person === knownPerson.id && Number(missed.note.fields.duration) === 0 && outCall;
    return [ok ? "pass" : "fail", `in: ${desc(inCall)}; missed: ${desc(missed)}; out (to ${OUTGOING}, a number no person holds): ${desc(outCall)}.`, [outShot]];
  });

  // 35 to 39, 44, 45: contacts, with the app open.
  openApp();
  await sleep(2_000);
  let bob;
  await check(35, async () => {
    bob = addContact("Bob Builder", "+16502530002");
    const p = await capturedUntil("a person for the contact", async () => (await people()).find((x) => x.note.fields.title === "Bob Builder"));
    const ok = phonesOf(p).includes("+16502530002");
    return [ok ? "pass" : "fail", `Person ${p.id} "${p.note.fields.title}", phones ${JSON.stringify(phonesOf(p))}.`, []];
  });

  await check(36, async () => {
    setPhone(bob, "+16502530022");
    const p = await capturedUntil("the new number", async () => (await people()).find((x) => x.note.fields.title === "Bob Builder" && phonesOf(x).includes("+16502530022")));
    return [phonesOf(p).includes("+16502530002") ? "fail" : "pass", `Person ${p.id} phones now ${JSON.stringify(phonesOf(p))}.`, []];
  });

  await check(37, async () => {
    await setDevice({ contacts_write: true, commands: ["upsert_contact", "delete_contact", "send_sms", "show_notification"] });
    const p = (await people()).find((x) => x.note.fields.title === "Bob Builder");
    await updateNote(p.id, (n) => {
      n.fields.title = "Robert Builder";
    });
    const r = await openThenNudge("the phone contact renamed", () => phoneContacts().includes("display_name=Robert Builder"));
    let second = "";
    if (r.nudged) {
      // Tell a stale contacts-write switch apart from a missing pull: rename once more, now that
      // the app has been back in front, and time it with the app left open.
      const q = (await people()).find((x) => x.note.fields.title === "Robert Builder");
      await updateNote(q.id, (n) => {
        n.fields.title = "Bobby Builder";
      });
      const t1 = Date.now();
      const again = await until("the second rename", () => phoneContacts().includes("display_name=Bobby Builder"), { timeout: 60_000, every: 1_000 }).then(() => elapsed(t1)).catch(() => null);
      second = again
        ? ` A second rename, made after the app had been back in front, reached the contact ${again} later with the app open: the open app pulls the person at once, but it acts on the contacts-write switch it had when it came to the front, so a switch turned on while it is open takes effect only after it leaves and returns.`
        : " A second rename, with the app open after coming back to the front, did not reach the contact within a minute either.";
      await updateNote(q.id, (n) => {
        n.fields.title = "Robert Builder";
      });
      await openThenNudge("the name back", () => phoneContacts().includes("display_name=Robert Builder"));
    }
    return [r.nudged ? "fail" : "pass", `Turned on contacts write and the commands; renamed the person on the server with the app open. ${r.nudged ? `The phone contact was not renamed within a minute; it was "Robert Builder" only after the app went to the background and came back (${r.took} in all).${second}` : `The phone contact was "Robert Builder" ${r.took} later.`}`, []];
  });

  let carla;
  await check(38, async () => {
    carla = await createNote("person", { title: "Carla Created", phones: ["+16502530005"] });
    const r = await openThenNudge("a phone contact", () => phoneContacts().includes("display_name=Carla Created"), 30_000, 60_000);
    const data = contactData("data1='+16502530005'");
    return [data.includes("+16502530005") ? "pass" : "fail", `Contact "Carla Created" appeared on the phone ${r.took} after the person was created, with its number${r.nudged ? " (after the app came back to the foreground: an open app does not pull server changes by itself)" : ""}.`, []];
  });

  await check(39, async () => {
    const kim = await note(knownPerson.id);
    const linkedBefore = Object.keys(linksOf(kim)).length;
    if (!linkedBefore) {
      return ["fail", `Kim Known's person note on the server holds no link to the phone contact added for it in check 29, so there is nothing to unlink. Person note: ${JSON.stringify(kim.note).slice(0, 400)}. Phone contact: ${contactData(`raw_contact_id=${knownContact}`).replace(/\n/g, " ").slice(0, 300)}`, []];
    }
    shell(`content delete --uri content://com.android.contacts/raw_contacts --where "_id=${knownContact}"`);
    const p = await capturedUntil("the person unlinked", async () => {
      const n = await note(knownPerson.id).catch(() => null);
      return n && Object.keys(linksOf(n)).length === 0 && n;
    });
    return [linkedBefore > 0 ? "pass" : "fail", `Deleted Kim Known's contact (linked to person ${knownPerson.id}) on the phone. The person is still there (title "${p.note.fields.title}"); links before ${linkedBefore}, after 0.`, []];
  });

  await check(44, async () => {
    const p = (await people()).find((x) => x.note.fields.title === "Robert Builder");
    addPhone(bob, "+16502530033");
    await updateNote(p.id, (n) => {
      n.fields.email = "robert@example.com";
    });
    await sleep(5_000);
    await foreground();
    const both = await until("both changes on the server", async () => {
      const n = await note(p.id);
      return n.note.fields.email === "robert@example.com" && phonesOf(n).includes("+16502530033") && n;
    }, { timeout: 120_000, every: 1_000 }).catch(() => null);
    const server = await note(p.id);
    await foreground();
    await sleep(10_000);
    const phone = contactData(`raw_contact_id=${bob}`);
    const conflicts = (await rows("from person")).filter((r) => /conflict/i.test(r.title ?? ""));
    const ok = both && phone.includes("+16502530033") && conflicts.length === 0;
    return [ok ? "pass" : "fail", `Server person: email=${server.note.fields.email}, phones=${JSON.stringify(phonesOf(server))}. Phone contact data: ${phone.replace(/\n/g, " ").slice(0, 300)}. Conflict copies: ${conflicts.length}.`, []];
  });

  await check(45, async () => {
    const p = (await people()).find((x) => x.note.fields.title === "Robert Builder");
    await srv.del(`/api/notes/${p.id}?base_version=${encodeURIComponent((await note(p.id)).version)}`);
    await foreground();
    await sleep(10_000);
    setName(bob, "Robert Renamed");
    await sleep(15_000);
    await foreground();
    await sleep(15_000);
    const back = (await people()).filter((x) => /Robert/.test(x.note.fields.title));
    const contact = phoneContacts().includes("Robert Renamed");
    return [back.length === 0 && contact ? "pass" : "fail", `Deleted person ${p.id} on the server, then renamed the contact on the phone. After 30 s: ${back.length} Robert person note(s) on the server; the contact is ${contact ? "still" : "no longer"} on the phone.`, []];
  });

  // 46, 47: device commands with the app closed and the screen off.
  shell(`pm grant ${PKG} android.permission.SEND_SMS`);
  await check(46, async () => {
    await setDevice({ sms_send: true });
    closeApp();
    screenOff();
    await sleep(2_000);
    const t0 = Date.now();
    const { dev, cmd } = await queueCommand("send_sms", { to: SELF, body: "Sent by a device command, check 46" });
    // The emulator's modem does not loop a text to its own number back, so "arrives" is the sent
    // row Android writes to the provider for the sending app.
    await until("the text to be sent", () => shellTry("content query --uri content://sms/sent --projection address:body").includes("Sent by a device command, check 46"), { timeout: 60_000, every: 500 });
    const arrived = elapsed(t0);
    const done = await until("the command done", async () => {
      const c = await commandStatus(dev.id, cmd.id);
      return ["done", "failed", "expired"].includes(c.status) && c;
    }, { timeout: 60_000 });
    return [done.status === "done" ? "pass" : "fail", `The text was sent (a sent row in the Telephony provider, to ${SELF}) ${arrived} after the command was queued; command ${cmd.id} is ${done.status}, result ${JSON.stringify(done.result)}.`, []];
  });

  await check(47, async () => {
    await setDevice({ sms_send: false });
    const { dev, cmd } = await queueCommand("send_sms", { to: SELF, body: "Must not be sent, check 47" });
    const done = await until("the command finished", async () => {
      const c = await commandStatus(dev.id, cmd.id);
      return ["done", "failed", "expired"].includes(c.status) && c;
    }, { timeout: 60_000 });
    await sleep(5_000);
    const sent = shellTry("content query --uri content://sms --projection body").includes("Must not be sent, check 47");
    const ok = done.status === "failed" && !sent && /SMS send/i.test(JSON.stringify(done.result));
    return [ok ? "pass" : "fail", `Command ${cmd.id} is ${done.status}, result ${JSON.stringify(done.result)}; text sent: ${sent}.`, []];
  });

  // 34. Restart.
  await check(34, async () => {
    await pw?.device.close().catch(() => {});
    pw = null;
    const refs = async () => {
      const all = [...(await notesOf("message")), ...(await notesOf("call"))];
      const by = {};
      for (const n of all) by[n.note.fields.device_ref] = (by[n.note.fields.device_ref] ?? 0) + 1;
      return { count: all.length, dupes: Object.entries(by).filter(([, c]) => c > 1) };
    };
    const before = await refs();
    await reboot();
    root();
    wakeAndUnlock();
    openApp();
    await sleep(30_000);
    await foreground();
    await sleep(15_000);
    const after = await refs();
    const ok = after.dupes.length === 0;
    return [ok ? "pass" : "fail", `Message and call notes before the restart: ${before.count}; after the restart and opening the app: ${after.count} (any new ones are captures still waiting to sync, such as check 46's sent SMS). Notes sharing a device_ref: ${JSON.stringify(after.dupes)}.`, [snap("34-after-restart")]];
  });

  // 42, 43: reinstall over the top; the version.
  await check(42, async () => {
    const before = (await local("GET", `/api/query?q=${encodeURIComponent("from note")}`)).body;
    const res = adb("install", "-r", apk);
    openApp();
    await sleep(8_000);
    const onboarding = L.find("Connect TheVault to your server");
    pw = null;
    const after = (await local("GET", `/api/query?q=${encodeURIComponent("from note")}`)).body;
    const same = JSON.parse(after).rows.length === JSON.parse(before).rows.length;
    return [!onboarding && same ? "pass" : "fail", `adb install -r of the same debug APK: ${res.trim().split("\n").pop()}. Still paired: ${!onboarding}; the app's notes: ${JSON.parse(after).rows.length} (before ${JSON.parse(before).rows.length}). No release APK or previous release was available, so this is the debug build over itself.`, [snap("42-after-update")]];
  });

  await check(43, async () => {
    const name = /versionName=(\S+)/.exec(shell(`dumpsys package ${PKG}`))?.[1];
    const conf = JSON.parse(readFileSync(join(repo, "apps/android/src-tauri/tauri.conf.json"), "utf8")).version;
    return ["partial", `App info version ${name}; tauri.conf.json version ${conf}. No release tag to compare with in this run (debug build).`, []];
  });

  // 9, 15, 40, 41, 22, 48: offline work and the 15-minute background jobs, in one wait.
  try {
    let offline = {};
    openApp();
    await sleep(3_000);
    await page();
    {
      airplane(true);
      await sleep(3_000);
      const create = await local("POST", "/api/notes?type=note", { note: { fields: { title: "Made offline for check 9" }, blocks: [] } });
      offline.create9 = `${create.status} ${create.body.slice(0, 120)}`;
      if (create.status === 201) {
        const made = JSON.parse(create.body);
        const target = JSON.parse((await local("GET", `/api/notes/${made.id}`)).body);
        target.note.fields.title = "Made and edited offline for check 9";
        const put = await local("PUT", `/api/notes/${made.id}`, { note: target.note, base_version: target.version });
        offline.note9 = made.id;
        offline.edit9 = put.status;
      }
      const s = await share({ text: "Shared offline for check fifteen" });
      offline.toast15 = s.toast;
      offline.shot15 = s.shot;
      offline.local15 = JSON.parse((await local("GET", `/api/query?q=${encodeURIComponent("from fleeting")}`)).body).rows.some((r) => /Shared offline/.test(r.title));
      await sleep(5_000);
      offline.server9 = offline.note9 ? (await rows("from note")).some((r) => r.id === offline.note9) : false;
      offline.server15 = (await notesOf("fleeting", (f, x) => textOf(x).includes("Shared offline for check fifteen"))).length > 0;
      await pw?.device.close().catch(() => {});
      pw = null;
      closeApp();
      airplane(false);
      await sleep(3_000);

      // 40: Wi-Fi and mobile data off, the modem on.
      shell("svc wifi disable");
      shell("svc data disable");
      await sleep(3_000);
      emu(`sms send ${KNOWN} Offline capture text check 40`);
      emu(`gsm call ${UNKNOWN}`);
      await sleep(4_000);
      emu(`gsm cancel ${UNKNOWN}`);
      await sleep(20_000);
      offline.server40 = (await notesOf("message", (f) => JSON.stringify(f).includes("Offline capture text check 40"))).length;
      shell("svc wifi enable");
      shell("svc data enable");
      offline.back = Date.now();
      record(40, offline.server40 === 0 ? "pass" : "fail", `Wi-Fi and mobile data off, modem on: received an SMS and a missed call; after 20 s the server holds ${offline.server40} note(s) for them.`, []);

      // 22, 48: no distributor.
      adb("uninstall", NTFY);
      await sleep(3_000);
      const n22 = await srv.post("/api/notifications", { title: "Check 22 fallback" });
      await setDevice({ commands: ["upsert_contact", "delete_contact", "send_sms", "show_notification"] });
      const q48 = await queueCommand("show_notification", { title: "Check 48 command", body: "From a device command", note: "" });
      offline.cmd48 = q48;
      offline.n22 = n22.id;
      screenOff();
    }
    const jobs = shellTry("dumpsys jobscheduler").split("\n").filter((l) => l.includes(PKG) && /TIME=|NET|URIS/.test(l)).join("\n");
    writeFileSync(join(logs, "jobs-before-wait.txt"), jobs);
    console.log(`waiting up to ${periodicWait / 60_000} min for the background jobs, without opening the app`);
    const waitStart = Date.now();
    const seen = {};
    while (Date.now() - waitStart < periodicWait) {
      if (seen[9] === undefined && offline.note9 && (await rows("from note")).some((r) => r.id === offline.note9 && r.title === "Made and edited offline for check 9")) seen[9] = Date.now();
      if (seen[15] === undefined && (await notesOf("fleeting", (f, x) => textOf(x).includes("Shared offline for check fifteen"))).length) seen[15] = Date.now();
      if (seen[41] === undefined && (await notesOf("message", (f) => JSON.stringify(f).includes("Offline capture text check 40"))).length) seen[41] = Date.now();
      try {
        if (seen[22] === undefined && shown("Check 22 fallback")) seen[22] = Date.now();
        if (seen[48] === undefined && shown("Check 48 command")) seen[48] = Date.now();
      } catch {}
      if ([...(offline.note9 ? [9] : []), 15, 41, 22, 48].every((k) => seen[k] !== undefined)) break;
      await sleep(10_000);
    }
    const at = (k) => (seen[k] === undefined ? null : ((seen[k] - offline.back) / 60_000).toFixed(1));
    writeFileSync(join(logs, "jobs-after-wait.txt"), shellTry("dumpsys jobscheduler").split("\n").filter((l) => l.includes(PKG) && /TIME=|NET|URIS/.test(l)).join("\n"));
    const waited = ((Date.now() - waitStart) / 60_000).toFixed(1);
    record(9, offline.edit9 === 200 && !offline.server9 && seen[9] ? "pass" : "fail", `In airplane mode, made a note in the app (POST answered ${offline.create9}) and edited it (PUT ${offline.edit9 ?? "not tried"}); not on the server while offline (${offline.server9}). With the app closed and the network back, it reached the server after ${at(9) ?? `more than ${waited}`} min.`, []);
    record(15, offline.toast15 === "Saved to TheVault inbox" && offline.local15 && !offline.server15 && seen[15] ? "pass" : "fail", `Shared in airplane mode: toast "${offline.toast15}", in the app's vault at once (${offline.local15}), not on the server while offline (${offline.server15}); on the server ${at(15) ?? `not within ${waited}`} min after the network came back.`, [offline.shot15]);
    record(41, seen[41] ? "pass" : "fail", `The SMS from check 40 reached the server ${at(41) ?? `not within ${waited}`} min after Wi-Fi and data came back.`, []);
    record(22, seen[22] ? "pass" : "fail", `ntfy uninstalled, app not opened since; the notification ${seen[22] ? `showed ${at(22)} min after the network came back` : `did not show within ${waited} min`}. Jobs before the wait: logs/jobs-before-wait.txt.`, []);
    const c48 = await commandStatus(offline.cmd48.dev.id, offline.cmd48.cmd.id);
    record(48, seen[48] ? "pass" : "fail", `show_notification queued with ntfy uninstalled; the notification ${seen[48] ? `showed ${at(48)} min later` : `did not show within ${waited} min`}; command status ${c48?.status}.`, []);
    wakeAndUnlock();
  } catch (e) {
    const shot = trySnap("offline-error");
    for (const n of [9, 15, 40, 41, 22, 48]) if (!results[n]) record(n, "fail", `The offline and background part of the run stopped: ${e.message}`, [shot]);
  }

  // 4, 23: revoke, clear storage, pair again refusing notifications.
  await check(4, async () => {
    const dev = await deviceRecord();
    await srv.del(`/api/devices/${dev.id}`);
    shell(`pm clear ${PKG}`);
    openApp();
    await L.waitFor("Connect TheVault to your server", { timeout: 30_000 });
    const scan = L.find(/scan pairing qr code/i);
    await pairWith(PASSWORD, "Emulator Pixel again");
    await L.waitFor(/^don.t allow$/i, { timeout: 30_000 });
    return ["partial", `Revoked ${dev.id}, cleared the app's storage, and the onboarding screen came back with its "${scan?.text}" button. The QR scan itself was not tested: the emulator's camera shows a virtual scene, not a pairing code. Paired again by password instead (check 23 continues).`, [snap("04-onboarding-again")]];
  });

  await check(23, async () => {
    await L.tap(/^don.t allow$/i);
    const dev = await until("the device on the server", async () => (await deviceRecord())?.name === "Emulator Pixel again" && (await deviceRecord()), { timeout: 30_000 });
    for (let i = 0; i < 8; i++) {
      if (await L.tapIf(/^finish$/i, 1_500)) break;
      await L.tapIf(/^skip$/i, 3_000);
    }
    closeApp();
    const n = await srv.post("/api/notifications", { title: "Check 23 none" });
    openApp();
    await sleep(10_000);
    closeApp();
    runJobs();
    await sleep(10_000);
    const none = !shown("Check 23 none");
    const granted = shell(`dumpsys package ${PKG}`).includes("android.permission.POST_NOTIFICATIONS: granted=true");
    shell(`pm grant ${PKG} android.permission.POST_NOTIFICATIONS`);
    const m = await srv.post("/api/notifications", { title: "Check 23 after grant" });
    openApp();
    let afterGrant = false;
    try {
      await timeNotification("Check 23 after grant", { timeout: 30_000 });
      afterGrant = true;
    } catch {}
    return [none && !granted ? "pass" : "fail", `Refused the permission: pairing completed (device "${dev.name}" on the server), and no Android notification showed for a server notification (${n.id}). After granting it, a new notification ${afterGrant ? "showed" : "did not show within 30 s"}.`, []];
  });

  record(5, "na", "Not possible in the emulator: the emulator's camera shows a virtual scene, so there is no way to hold a QR code up to it.", []);
}

function writeReport() {
  const run = existsSync(join(out, "run.json")) ? JSON.parse(readFileSync(join(out, "run.json"), "utf8")) : {};
  const labels = { pass: "Pass", fail: "Fail", na: "Not possible in emulator", partial: "Partial" };
  const lines = [
    "# Android manual checks on the emulator",
    "",
    `Run ${run.started ?? ""} by \`apps/android/emulator/checks.mjs\`. Device: ${run.device?.model ?? "?"}, Android ${run.device?.release ?? "?"} (API ${run.device?.sdk ?? "?"}). APK: \`${(run.apk ?? apk).replace(`${repo}/`, "")}\`.`,
    "",
    "| # | Check | Result | What happened | Evidence |",
    "|---|-------|--------|---------------|----------|",
  ];
  for (const n of Object.keys(TITLES).map(Number).sort((a, b) => a - b)) {
    if (only && !only.has(n)) continue;
    const r = results[n] ?? { status: "not run", note: "The run stopped before this check.", evidence: [] };
    const cell = (s) => String(s).replace(/\|/g, "\\|").replace(/\n/g, " ");
    lines.push(`| ${n} | ${cell(TITLES[n].doing.slice(0, 80))}${TITLES[n].doing.length > 80 ? "…" : ""} | ${labels[r.status] ?? r.status} | ${cell(r.note)} | ${r.evidence.map((e) => `\`${e}\``).join(", ")} |`);
  }
  const counts = {};
  for (const r of Object.values(results)) counts[r.status] = (counts[r.status] ?? 0) + 1;
  lines.push("", `Totals: ${Object.entries(counts).map(([k, v]) => `${labels[k] ?? k} ${v}`).join(", ")}.`, "");
  writeFileSync(join(out, "RESULTS.generated.md"), lines.join("\n"));
}

// Playwright's adb connection emits an unhandled error when adb restarts (a reboot, adb root);
// the next page() reconnects, so log it rather than lose the run.
process.on("uncaughtException", (e) => console.error(`ignored: ${e.message}`));

try {
  await main();
} catch (e) {
  console.error(`run stopped: ${e.stack ?? e}`);
  process.exitCode = 1;
} finally {
  try {
    airplane(false);
    shellTry("svc wifi enable");
    shellTry("svc data enable");
    L.logcatSave(join(logs, "logcat.txt"));
  } catch {}
  await pw?.device.close().catch(() => {});
  writeReport();
  for (const c of children) await L.stop(c);
  const failed = Object.values(results).filter((r) => r.status === "fail").length;
  console.log(`${Object.keys(results).length} checks recorded, ${failed} failed; see ${join(out, "RESULTS.generated.md")}`);
  if (failed) process.exitCode = 1;
}
