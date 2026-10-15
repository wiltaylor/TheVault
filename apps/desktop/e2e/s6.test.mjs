// S6: the desktop app end to end, under Xvfb with WebKitGTK through tauri-driver (D2, D18, D19,
// D21, D26, D27). The app starts unpaired, pairs with a testkit home server by password, shows
// the server's notes from its local vault, takes an edit while the server is down, and the edit
// reaches the server once it is back. Started again, it reads its token from the keyring and
// syncs without pairing. `thevault --capture` opens the capture dialog (D12).

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { Desktop, HomeServer, appBinary, removeRoot, tempRoot } from "./harness.mjs";
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
const badge = () => app.run("const b = document.getElementById('thevault-sync'); return b ? { phase: b.dataset.phase, pending: Number(b.dataset.pending), text: b.textContent } : null;");

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

  // The server's note was pulled into the local vault and shows.
  await app.navigate(`${ORIGIN}/notes/${created.id}`);
  await waitFor("the pulled note's title", async () => (await app.text("[aria-label='Title']")) === "Written on the server");
  await waitFor("the synced badge", async () => (await badge())?.phase === "synced");

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
  const offline = await waitFor("the offline badge with the change waiting", async () => {
    const b = await badge();
    return b?.phase === "offline" && b.pending === 1 ? b : null;
  });
  assert.match(offline.text, /Offline · 1 change waiting/);

  // The server comes back on the same address, and the change reaches it.
  await home.start();
  await waitFor(
    "the edit on the server",
    async () => (await home.api("GET", `/api/notes/${created.id}`)).note.fields.title === "Written on the server, edited offline",
    60_000,
  );
  await waitFor("the synced badge with nothing waiting", async () => {
    const b = await badge();
    return b?.phase === "synced" && b.pending === 0;
  });

  // Started again, the app is still paired: it reads its token from the keyring and syncs.
  await app.quit();
  const synced = await home.api("GET", `/api/notes/${created.id}`);
  await home.api("PUT", `/api/notes/${created.id}`, {
    note: { fields: { title: "Renamed on the server" }, blocks: [] },
    base_version: synced.version,
  });
  app = await launch();
  await waitFor("the web client, not setup", inWebClient);
  await waitFor("a sync with the stored token", async () => (await badge())?.phase === "synced");
  await app.navigate(`${ORIGIN}/notes/${created.id}`);
  await waitFor("the server's rename", async () => (await app.text("[aria-label='Title']")) === "Renamed on the server");
}

test("thevault --capture opens the capture dialog", async () => {
  await app?.quit();
  app = await launch(["--capture"]);
  await waitFor("the web client", inWebClient);
  await waitFor("the capture dialog", () => app.run("return !!document.querySelector('[aria-label=\"Capture text\"]')"));
});
