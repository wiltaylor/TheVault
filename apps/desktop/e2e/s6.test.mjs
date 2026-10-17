// S6: the desktop app end to end, under Xvfb with WebKitGTK through tauri-driver (D2, D18, D19,
// D21, D26, D27). The app starts unpaired, pairs with a testkit home server by password, shows
// the server's notes from its local vault, takes an edit while the server is down, and the edit
// reaches the server once it is back. Started again, it reads its token from the keyring and
// syncs without pairing. `thevault --capture` opens the capture dialog (D12).
//
// The desktop extras (D9, D12, D27): a notification created on the server updates the tray badge
// and raises a desktop notification; reading it through the API closes it; closing it on the
// desktop marks it read, and clicking it opens its note. A note changed on the server is pulled
// within seconds while the app is open. A newer release is offered in the window. A signed
// AppImage published on the server while the app runs is announced, downloaded and verified at
// once, and the window offers to restart to update (D27).

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { Desktop, HomeServer, appBinary, removeRoot, tempRoot, vaultBinary } from "./harness.mjs";
import { Session, waitFor } from "./webdriver.mjs";

const ORIGIN = "vault://localhost";

let root;
let home;
let desktop;
let app;

const launch = (args = []) =>
  Session.start(desktop.webdriver, {
    browserName: "wry",
    "tauri:options": { application: appBinary(), args },
  });

const path = async () => new URL(await app.url()).pathname;
/** Whether the window shows the web client, which sends `/` on to a page of its own choosing. */
const inWebClient = async () => !(await path()).startsWith("/_desktop/") && (await badge()) !== null;
/** The sync icon beside Archive in the sidebar footer: its colour state, phase, waiting count and words. */
const badge = () =>
  app.run(
    "const b = document.querySelector('.sidebar-footer .sync-button'); return b ? { state: b.dataset.syncState, phase: b.dataset.phase, pending: Number(b.dataset.queued), text: b.getAttribute('aria-label') } : null;",
  );
/** Record every state the sync icon shows from now on, in `window.__syncStates`. */
const recordStates = () =>
  app.run(`
    window.__syncStates = [];
    const note = () => {
      const s = document.querySelector('.sidebar-footer .sync-button')?.dataset.syncState;
      if (s && window.__syncStates.at(-1) !== s) window.__syncStates.push(s);
    };
    note();
    window.__syncObserver?.disconnect();
    window.__syncObserver = new MutationObserver(note);
    window.__syncObserver.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-sync-state'] });
  `);
const recorded = () => app.run("return window.__syncStates;");

before(async () => {
  root = tempRoot();
  home = new HomeServer(root);
  await home.start();
  desktop = new Desktop(root);
  await desktop.start();
});

after(async () => {
  await app?.quit().catch(() => {});
  await desktop?.stop();
  await home?.stop();
  if (root) removeRoot(root);
});

/** What the window shows and what the app says about sync, for a failure's message. */
async function diagnose() {
  try {
    const url = await app.url();
    const text = await app.run("return document.body ? document.body.innerText.slice(0, 2000) : ''");
    const status = await app.run("return fetch('/_desktop/status').then(r => r.text());");
    return `url ${url}\nstatus ${status}\npage text:\n${text}`;
  } catch (e) {
    return `no diagnosis: ${e.message}`;
  }
}

test("the app pairs, shows notes, edits offline and syncs the edit once the server is back", async () => {
  try {
    await scenario();
  } catch (e) {
    e.message += `\n${await diagnose()}`;
    throw e;
  }
});

async function scenario() {
  const created = await home.api("POST", "/api/notes?type=note", {
    note: { fields: { title: "Written on the server" }, blocks: [] },
  });

  app = await launch();
  await waitFor("the setup page", async () => (await path()) === "/_desktop/setup");

  // Pairing with a wrong password says so and stays on setup.
  await app.type(await app.find("#server"), home.url);
  await app.type(await app.find("#password"), "not the password");
  await app.click(await app.find("#submit"));
  await waitFor("the wrong password error", async () => /wrong password/i.test((await app.text("#error")) ?? ""));
  assert.equal(await path(), "/_desktop/setup");

  await app.run("document.getElementById('password').value = ''");
  await app.type(await app.find("#password"), home.password);
  await app.click(await app.find("#submit"));
  await waitFor("the web client after the first sync", inWebClient, 60_000);

  // The token is in the keyring, not in the device file.
  const deviceFile = JSON.parse(readFileSync(join(desktop.dataDir, "device.json"), "utf8"));
  assert.equal(deviceFile.server_url, home.url);
  assert.ok(!JSON.stringify(deviceFile).includes("token"), "the device file holds no token");
  const devices = await home.api("GET", "/api/devices");
  const listed = JSON.stringify(devices);
  assert.ok(listed.includes(deviceFile.device_id), `the server lists the device: ${listed}`);

  // Settings > Devices in the app lists the home server's devices, this one marked, read-only:
  // the device table is the server's, not the local vault's.
  await app.navigate(`${ORIGIN}/settings/devices`);
  const card = `[data-device='${deviceFile.device_id}']`;
  await waitFor("this device in Settings > Devices", async () => /This device/.test((await app.text(`${card} h2`)) ?? ""));
  assert.equal(await app.run(`return document.querySelector("${card} input[id$='-name']").matches(':disabled');`), true);
  assert.equal(await app.run("return [...document.querySelectorAll('.settings-tabs a')].some((a) => a.textContent === 'API tokens');"), false);

  // The server's note was pulled into the local vault and shows.
  await app.navigate(`${ORIGIN}/notes/${created.id}`);
  await waitFor("the pulled note's title", async () => (await app.text("[aria-label='Title']")) === "Written on the server");
  // The sync icon is green after the first sync, and no sync toast shows any more.
  await waitFor("the green sync icon", async () => (await badge())?.state === "synced");
  assert.match((await badge()).text, /^Synced/);
  assert.equal(await app.run("return document.getElementById('thevault-sync');"), null);

  // The server goes away; the edit is saved locally and waits.
  await home.stop();
  await app.click(await app.find("[aria-label='Title']"));
  await app.run(`
    const el = document.querySelector("[aria-label='Title']");
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  `);
  await app.type(await app.find("[aria-label='Title']"), ", edited offline");
  await waitFor("the local save", async () => (await app.text("[data-testid='save-status']")) === "Saved");
  // Typing can reach the local vault as one save or several, depending on how fast it goes.
  const offline = await waitFor("the red sync icon with the change waiting", async () => {
    const b = await badge();
    return b?.state === "error" && b.phase === "offline" && b.pending >= 1 ? b : null;
  });
  assert.match(offline.text, /^Sync failed: cannot reach the server — retrying · \d+ changes? waiting$/);

  // The server comes back on the same address, and the change reaches it.
  await recordStates();
  await home.start();
  await waitFor(
    "the edit on the server",
    async () => (await home.api("GET", `/api/notes/${created.id}`)).note.fields.title === "Written on the server, edited offline",
    60_000,
  );
  await waitFor("the green sync icon with nothing waiting", async () => {
    const b = await badge();
    return b?.state === "synced" && b.pending === 0;
  });
  const recovered = await recorded();
  assert.equal(recovered[0], "error", `from red: ${recovered}`);
  assert.equal(recovered.at(-1), "synced", `to green: ${recovered}`);

  // Typing straight on in the same open editor, without opening the note again, after the push
  // took the server's result back into the local vault with a version of its own, is saved and
  // reaches the server too.
  await app.click(await app.find("[aria-label='Title']"));
  await app.run(`
    const el = document.querySelector("[aria-label='Title']");
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  `);
  await app.type(await app.find("[aria-label='Title']"), " and again");
  await waitFor(
    "the typing after the sync on the server",
    async () => (await home.api("GET", `/api/notes/${created.id}`)).note.fields.title === "Written on the server, edited offline and again",
    60_000,
  );
  assert.equal(await app.run("return !!document.querySelector(\"[data-testid='conflict-view']\");"), false, "no conflict view");
  assert.equal(await app.text("[data-testid='save-status']"), "Saved");
  await waitFor("the green sync icon after typing on", async () => {
    const b = await badge();
    return b?.state === "synced" && b.pending === 0;
  });

  // An edit made online shows yellow while it waits for its round, then green.
  await app.navigate(`${ORIGIN}/notes/${created.id}`);
  await waitFor("the note again", async () => (await app.text("[aria-label='Title']")) === "Written on the server, edited offline and again");
  await waitFor("the green sync icon on the note", async () => (await badge())?.state === "synced");
  await recordStates();
  await app.click(await app.find("[aria-label='Title']"));
  await app.run(`
    const el = document.querySelector("[aria-label='Title']");
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  `);
  await app.type(await app.find("[aria-label='Title']"), "!");
  await waitFor("the local save of the online edit", async () => (await app.text("[data-testid='save-status']")) === "Saved");
  let onServer;
  await waitFor(
    "the edit on the server again",
    async () => {
      onServer = (await home.api("GET", `/api/notes/${created.id}`)).note.fields.title;
      return onServer === "Written on the server, edited offline and again!";
    },
    60_000,
  ).catch(async (e) => {
    const local = await app.run(`return fetch('/api/notes/${created.id}').then(r => r.text());`);
    const saved = await app.text("[data-testid='save-status']");
    e.message += ` (the server's title: ${JSON.stringify(onServer)}; save status ${JSON.stringify(saved)}; local note ${local.slice(0, 400)})`;
    throw e;
  });
  await waitFor("the green sync icon after the edit", async () => {
    const b = await badge();
    return b?.state === "synced" && b.pending === 0;
  });
  const online = await recorded();
  assert.deepEqual(online.slice(-2), ["syncing", "synced"], `yellow then green: ${online}`);

  // Started again, the app is still paired: it reads its token from the keyring and syncs.
  await app.quit();
  const synced = await home.api("GET", `/api/notes/${created.id}`);
  await home.api("PUT", `/api/notes/${created.id}`, {
    note: { fields: { title: "Renamed on the server" }, blocks: [] },
    base_version: synced.version,
  });
  app = await launch();
  await waitFor("the web client, not setup", inWebClient);
  await waitFor("a sync with the stored token", async () => (await badge())?.state === "synced");
  await app.navigate(`${ORIGIN}/notes/${created.id}`);
  await waitFor("the server's rename", async () => (await app.text("[aria-label='Title']")) === "Renamed on the server");
}

test("thevault --capture opens the capture dialog", async () => {
  await app?.quit();
  app = await launch(["--capture"]);
  await waitFor("the web client", inWebClient);
  await waitFor("the capture dialog", () => app.run("return !!document.querySelector('[aria-label=\"Capture text\"]')"));
});

/** The app's own status, from inside the window. */
/** Whether the update prompt shows, and its text when it does. */
const updatePrompt = () =>
  app.run("const p = document.getElementById('thevault-update'); return p && getComputedStyle(p).display !== 'none' ? p.textContent : null;");

const status = async () => JSON.parse(await app.run("return fetch('/_desktop/status').then(r => r.text());"));

test("a server notification updates the tray badge, shows on the desktop, and closes when read", async () => {
  try {
    await notifications();
  } catch (e) {
    e.message += `\nnotification daemon log ${JSON.stringify(desktop.notifyd.log)}\n${await diagnose()}`;
    throw e;
  }
});

async function notifications() {
  const notifyd = desktop.notifyd;
  await waitFor("the live link to the home server", async () => (await status()).notifications.connected);
  assert.equal((await status()).tray.unread, 0);

  const note = await home.api("POST", "/api/notifications", { title: "Build finished", body: "All green" });
  const [raised] = await waitFor("the desktop notification", () => notifyd.raised("Build finished").length && notifyd.raised("Build finished"));
  assert.equal(raised.body, "All green");
  assert.equal(raised.app, "TheVault");
  const badged = await waitFor("the tray badge", async () => {
    const tray = (await status()).tray;
    return tray.unread === 1 ? tray : null;
  });
  assert.deepEqual(badged, { tray: true, unread: 1, title: "1", tooltip: "TheVault: 1 unread notification" });

  // Read through the API, as the web app or a script would: the desktop notification closes.
  await home.api("POST", `/api/notifications/${note.id}/read`);
  await waitFor("the desktop notification to close", () => notifyd.closed(raised.notify));
  await waitFor("the badge to clear", async () => (await status()).tray.unread === 0);
  assert.equal(notifyd.raised("Build finished").length, 1, "raised once");

  // Closed on the desktop, it is read on the server.
  const second = await home.api("POST", "/api/notifications", { title: "Second" });
  const [shown] = await waitFor("the second notification", () => notifyd.raised("Second").length && notifyd.raised("Second"));
  await waitFor("the badge", async () => (await status()).tray.unread === 1);
  notifyd.dismiss(shown.notify);
  await waitFor("the server to mark it read", async () => (await home.api("GET", "/api/notifications?unread=true")).unread === 0);
  const listed = await home.api("GET", "/api/notifications");
  assert.ok(listed.items.find((n) => n.id === second.id).read, "read on the server");
  await waitFor("the badge to clear", async () => (await status()).tray.unread === 0);

  // Clicked, it opens its note in the window, and is read.
  const target = await home.api("POST", "/api/notes?type=note", { note: { fields: { title: "Opened from a notification" }, blocks: [] } });
  await waitFor("the note to sync", async () => {
    const res = await app.run(`return fetch('/api/notes/${target.id}').then(r => r.status);`);
    if (res !== 200) await app.run("return fetch('/_desktop/sync', { method: 'POST' }).then(() => 0);");
    return res === 200;
  }, 60_000);
  await home.api("POST", "/api/notifications", { title: "Open me", note: target.id });
  const [clickable] = await waitFor("the third notification", () => notifyd.raised("Open me").length && notifyd.raised("Open me"));
  notifyd.click(clickable.notify);
  await waitFor("the note in the window", async () => (await path()) === `/notes/${target.id}`);
  await waitFor("the note's title", async () => (await app.text("[aria-label='Title']")) === "Opened from a notification");
  await waitFor("the server to mark it read", async () => (await home.api("GET", "/api/notifications?unread=true")).unread === 0);
}

// Ticket 82: the timed round runs once a minute, so a change seen within a few seconds, twice,
// came through the live link's note.changed.
test("a note changed on the server is pulled within seconds while the app is open", async () => {
  await waitFor("the live link to the home server", async () => (await status()).notifications.connected);
  const local = (id) => app.run(`return fetch('/api/notes/${id}').then(r => r.ok ? r.json() : null).then(n => n && n.note.fields.title);`);
  const made = await home.api("POST", "/api/notes?type=note", { note: { fields: { title: "Made on the server" }, blocks: [] } });
  await waitFor("the new note in the local vault", async () => (await local(made.id)) === "Made on the server", 8_000);
  const current = await home.api("GET", `/api/notes/${made.id}`);
  await home.api("PUT", `/api/notes/${made.id}`, {
    note: { fields: { title: "Renamed on the server" }, blocks: [] },
    base_version: current.version,
  });
  await waitFor("the rename in the local vault", async () => (await local(made.id)) === "Renamed on the server", 8_000);
});

test("a thevault://note link opens the note", async () => {
  const notes = await home.api("POST", "/api/notes?type=note", { note: { fields: { title: "Linked" }, blocks: [] } });
  await app.run("return fetch('/_desktop/sync', { method: 'POST' }).then(() => 0);");
  await waitFor("the note to sync", async () => (await app.run(`return fetch('/api/notes/${notes.id}').then(r => r.status);`)) === 200, 60_000);
  await app.quit();
  app = await launch([`thevault://note/${notes.id}`]);
  await waitFor("the linked note", async () => (await path()) === `/notes/${notes.id}`);
  await waitFor("the linked note's title", async () => (await app.text("[aria-label='Title']")) === "Linked");
});

test("Settings > General's About shows the app's version from tauri.conf.json and the home server's", async () => {
  const conf = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const server = (await home.api("GET", "/api/version")).server_version;
  await app.navigate(`${ORIGIN}/settings`);
  const about = (key) => app.text(`[data-about='${key}']`);
  await waitFor("the app's version", async () => (await about("app")) === `TheVault ${conf.version}`);
  assert.equal(await about("platform"), "Linux AppImage");
  assert.equal(await about("build"), "debug");
  await waitFor("the home server's version", async () => (await about("server")) === server);
  // The update check runs when asked, whatever the timer says.
  const checks = (await status()).updater.checks;
  await app.click(await app.find("[aria-label='Updates'] button"));
  await waitFor("the asked-for check", async () => (await status()).updater.checks > checks, 60_000);
  await waitFor("up to date", async () => (await about("updates")) === "Up to date");
});

test("a newer release is offered in the window, and Later hides the offer", async () => {
  await app.quit();
  desktop.releases.tag = "v99.0.0";
  app = await launch();
  await waitFor("the web client", inWebClient);
  assert.equal(await updatePrompt(), null, "nothing offered before the check");
  const offer = await waitFor("the update prompt", updatePrompt, 60_000);
  assert.match(offer, /TheVault 99\.0\.0 is available/);
  assert.equal((await status()).update.version, "99.0.0");
  await app.click(await app.find("#thevault-update [data-action='later']"));
  await waitFor("the prompt to hide", async () => (await updatePrompt()) === null);
  desktop.releases.tag = "v0.1.0";
});

test("an AppImage the paired server publishes is offered before GitHub's release", async () => {
  await app.quit();
  // GitHub has 99.0.0 and the server 98.0.0: the server's wins because the app asks it first.
  desktop.releases.tag = "v99.0.0";
  const clients = join(home.vault, ".vault", "clients");
  const arch = process.arch === "arm64" ? "aarch64" : "x86_64";
  const filename = `TheVault_98.0.0_${arch}.AppImage`;
  mkdirSync(clients, { recursive: true });
  writeFileSync(join(clients, filename), "appimage");
  const entry = {
    platform: "linux",
    kind: "appimage",
    arch,
    version: "98.0.0",
    build: "release",
    size: 8,
    sha256: "0".repeat(64),
    filename,
    published: 0,
  };
  writeFileSync(join(clients, "manifest.json"), JSON.stringify({ clients: [entry] }));
  try {
    app = await launch();
    await waitFor("the web client", inWebClient);
    const offer = await waitFor("the update prompt", updatePrompt, 60_000);
    assert.match(offer, /TheVault 98\.0\.0 is available from your server/);
    const update = (await status()).update;
    assert.equal(update.source, "server");
    assert.equal(update.url, `${home.url}/api/clients/${filename}`);
    await app.click(await app.find("#thevault-update [data-action='later']"));
  } finally {
    rmSync(clients, { recursive: true, force: true });
    desktop.releases.tag = "v0.1.0";
  }
});

test("a signed AppImage published while the app runs is downloaded at once and offered as a restart", async () => {
  await app.quit();
  app = await launch();
  await waitFor("the web client", inWebClient);
  // The first check, ten seconds after start, finds nothing: the server publishes no AppImage.
  await waitFor("the first update check", async () => (await status()).updater.checks >= 1, 60_000);
  let s = await status();
  assert.equal(s.update, null, JSON.stringify(s.update));
  assert.equal(s.updater.install, "appimage");
  const checks = s.updater.checks;

  // Publish a signed 97.0.0 with the CLI, as `just publish-clients` does. The next periodic
  // check is hours away, so only the server's clients.updated can start the download.
  const build = join(root, "build");
  mkdirSync(build, { recursive: true });
  const arch = process.arch === "arm64" ? "aarch64" : "x86_64";
  const image = join(build, `TheVault_97.0.0_${arch}.AppImage`);
  writeFileSync(image, "\x7fELF TheVault 97.0.0");
  desktop.updaterKey.sign(image, "97.0.0");
  const clients = join(home.vault, ".vault", "clients");
  execFileSync(vaultBinary(), ["clients", "publish", "--dir", clients, image], { stdio: "ignore" });
  try {
    const offer = await waitFor(
      "the restart prompt",
      async () => {
        const text = await updatePrompt();
        return text && text.includes("ready") ? text : null;
      },
      60_000,
    );
    assert.match(offer, /TheVault 97\.0\.0 is ready \u2014 restart to update/);
    s = await status();
    assert.equal(s.update.version, "97.0.0");
    assert.equal(s.update.ready, true);
    assert.equal(s.updater.ready, "97.0.0");
    await waitFor("the check to finish", async () => (await status()).updater.checks > checks);
    s = await status();
    assert.equal(s.updater.checks, checks + 1, "one check, started by the announcement");
    assert.match(s.tray.tooltip, /97\.0\.0 is ready/);
    // Installing replaces the AppImage; the test leaves it, and the restart, to a real install.
    assert.equal(readFileSync(desktop.appimage, "utf8"), "\x7fELF the installed AppImage");
  } finally {
    rmSync(clients, { recursive: true, force: true });
  }
});
