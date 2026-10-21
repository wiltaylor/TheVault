// The first sync's progress on the setup page (ticket 144; D19, D20, D29). A home server holds
// hundreds of notes and tens of megabytes of assets, and the app reaches it through a proxy that
// slows what the server sends, so the first sync takes several seconds. After pairing, the setup
// page shows a bar with "Notes N / M", "Files X / Y (a / b MB)" and "about T left", updated at
// least once a second, and opens the web client when the sync finishes.
//
// A screenshot of the page part way through is written to the test's temporary folder, and to
// FIRST_SYNC_SHOT when it is set.

import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { after, before, test } from "node:test";
import { Desktop, HomeServer, appBinary, removeRoot, tempRoot, throttlingProxy } from "./harness.mjs";
import { Session, waitFor } from "./webdriver.mjs";

const NOTES = 400;
const ASSETS = 6;
const ASSET_BYTES = 4 * 1024 * 1024;
/** What the proxy lets through from the server, in bytes a second. */
const RATE = 2 * 1024 * 1024;

let root;
let home;
let proxy;
let desktop;
let app;

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
  await proxy?.stop();
  await home?.stop();
  if (root) removeRoot(root);
});

/** Fill the home server: `NOTES` notes and `ASSETS` assets of `ASSET_BYTES` each. */
async function fill() {
  for (let i = 0; i < ASSETS; i++) {
    const res = await fetch(`${home.url}/api/assets?filename=photo-${i}.jpg`, {
      method: "POST",
      headers: { authorization: `Bearer ${home.adminToken}` },
      body: Buffer.alloc(ASSET_BYTES, i + 1),
    });
    assert.equal(res.status, 201, await res.text());
  }
  for (let start = 0; start < NOTES; start += 20) {
    await Promise.all(
      Array.from({ length: Math.min(20, NOTES - start) }, (_, k) =>
        home.api("POST", "/api/notes?type=note", { note: { fields: { title: `Note ${start + k}` }, blocks: [] } }),
      ),
    );
  }
}

/** The setup page's first-sync panel, as the user reads it. */
const panel = () =>
  app.run(`
    const s = document.getElementById("first-sync");
    if (!s || s.hidden) return null;
    return {
      notes: document.getElementById("sync-notes").textContent,
      files: document.getElementById("sync-files").textContent,
      eta: document.getElementById("sync-eta").textContent,
      bar: document.getElementById("sync-bar").value,
      error: document.getElementById("sync-error").textContent,
    };
  `);

test("the setup page shows the first sync's progress and opens the web client when it ends", async () => {
  await fill();
  proxy = await throttlingProxy(home.address, RATE);

  app = await Session.start(desktop.webdriver, {
    browserName: "wry",
    "tauri:options": { application: appBinary(), args: [] },
  });
  await waitFor("the setup page", async () => new URL(await app.url()).pathname === "/_desktop/setup");
  await app.type(await app.find("#server"), `http://${proxy.address}`);
  await app.type(await app.find("#password"), home.password);
  await app.click(await app.find("#submit"));

  // Part way through the files: notes counted, megabytes arriving, time left estimated.
  const totalMb = ((ASSETS * ASSET_BYTES) / (1024 * 1024)).toFixed(1);
  const mid = await waitFor(
    "the progress part way through the files",
    async () => {
      const p = await panel();
      const m = p && /^(\d+) \/ (\d+) \(([\d.]+) \/ ([\d.]+) MB\)$/.exec(p.files);
      return m && Number(m[3]) > 0 && Number(m[3]) < Number(m[4]) && /^about \d+ (s|min) left$/.test(p.eta) ? p : null;
    },
    120_000,
  );
  const [, , files, , mb] = /^(\d+) \/ (\d+) \(([\d.]+) \/ ([\d.]+) MB\)$/.exec(mid.files);
  assert.equal(Number(files), ASSETS, mid.files);
  assert.equal(mb, totalMb, mid.files);
  const [done, total] = mid.notes.split(" / ").map(Number);
  assert.equal(done, total, `the notes are all in before the files: ${mid.notes}`);
  assert.ok(total > NOTES, `every note and definition file: ${mid.notes}`);
  assert.ok(mid.bar > 0 && mid.bar < 1, `the bar: ${mid.bar}`);
  assert.equal(mid.error, "");

  const shot = await app.screenshot();
  const path = join(root, "first-sync-progress.png");
  writeFileSync(path, shot);
  if (process.env.FIRST_SYNC_SHOT) {
    mkdirSync(dirname(process.env.FIRST_SYNC_SHOT), { recursive: true });
    copyFileSync(path, process.env.FIRST_SYNC_SHOT);
  }

  // It updates at least once a second while the files come in.
  const seen = [];
  for (let i = 0; i < 6; i++) {
    const p = await panel();
    if (!p) break;
    seen.push(p.files);
    await new Promise((r) => setTimeout(r, 1000));
  }
  const changes = seen.slice(1).filter((f, i) => f !== seen[i]).length;
  assert.ok(changes >= seen.length - 2, `the files line moved each second: ${JSON.stringify(seen)}`);

  // Finished, the page opens the web client, with every asset in the local vault.
  await waitFor(
    "the web client after the first sync",
    async () => !new URL(await app.url()).pathname.startsWith("/_desktop/"),
    180_000,
  );
  const status = await app.run("return fetch('/_desktop/status').then((r) => r.json());");
  assert.equal(status.progress.stage, "done", JSON.stringify(status.progress));
  assert.equal(status.progress.bytes_done, ASSETS * ASSET_BYTES);
  assert.equal(status.progress.files_done, ASSETS);
});
