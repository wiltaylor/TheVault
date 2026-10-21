// Joining another vault starts fresh on the desktop, as on Android (ticket 145; D20, D29). The
// app pairs with home server A and syncs it. Paired again from the setup page, with server B,
// which holds another vault, it moves its local vault aside to vault.prev-<ms> beside it, opens a
// fresh one, takes B's vault whole and sends B nothing of A's; Settings > Devices says so. Then
// B's vault changes under the same pairing (its vault id is not the one the app recorded), and
// the next round starts fresh again and syncs it.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { Desktop, HomeServer, appBinary, removeRoot, tempRoot } from "./harness.mjs";
import { Session, waitFor } from "./webdriver.mjs";

const ORIGIN = "vault://localhost";

let root;
let a;
let b;
let desktop;
let app;

before(async () => {
  root = tempRoot();
  a = new HomeServer(join(root, "a"));
  await a.start();
  b = new HomeServer(join(root, "b"));
  await b.start();
  desktop = new Desktop(join(root, "desktop"));
  await desktop.start();
});

after(async () => {
  await app?.quit().catch(() => {});
  await desktop?.stop();
  await a?.stop();
  await b?.stop();
  if (root) removeRoot(root);
});

const path = async () => new URL(await app.url()).pathname;

/** The local vault folders fresh starts moved aside, oldest first. */
const previous = () =>
  readdirSync(desktop.dataDir)
    .filter((n) => n.startsWith("vault.prev-"))
    .sort()
    .map((n) => join(desktop.dataDir, n));

/** The id of the vault `server` holds, as its sync info says. */
const vaultId = async (server) => (await (await fetch(`${server.url}/api/sync/info`)).json()).vault_id;

/** What the local vault answers for note `id`: its status code. */
const localNote = (id) => app.run(`return fetch('/api/notes/${id}').then((r) => r.status);`);

/** What `GET /api/app` says about the last fresh start. */
const freshStart = () => app.run("return fetch('/api/app').then((r) => r.json()).then((a) => a.fresh_start);");

const note = (server, title) =>
  server.api("POST", "/api/notes?type=note", { note: { fields: { title }, blocks: [] } });

/** Pair from the setup page with `server` by password, and wait for its first sync. */
async function pairWith(server) {
  await app.navigate(`${ORIGIN}/_desktop/setup`);
  await waitFor("the setup page", async () => (await path()) === "/_desktop/setup");
  await app.type(await app.find("#server"), server.url);
  await app.type(await app.find("#password"), server.password);
  await app.click(await app.find("#submit"));
  await waitFor("the web client after the first sync", async () => !(await path()).startsWith("/_desktop/"), 60_000);
}

/** The notice Settings > Devices shows after a fresh start. */
async function notice() {
  await app.navigate(`${ORIGIN}/settings/devices`);
  return waitFor("the fresh start notice", () => app.text("[data-testid='fresh-start']"));
}

async function diagnose() {
  try {
    const status = await app.run("return fetch('/_desktop/status').then(r => r.text());");
    return `url ${await app.url()}\nstatus ${status}\nfolders ${readdirSync(desktop.dataDir).join(", ")}`;
  } catch (e) {
    return `no diagnosis: ${e.message}`;
  }
}

test("a pairing or a handshake with another vault starts a fresh local vault", async () => {
  try {
    await scenario();
  } catch (e) {
    e.message += `\n${await diagnose()}`;
    throw e;
  }
});

async function scenario() {
  const fromA = await note(a, "Kept by vault A");
  const fromB = await note(b, "Kept by vault B");
  const vaultA = await vaultId(a);
  const vaultB = await vaultId(b);
  assert.ok(vaultA && vaultB && vaultA !== vaultB, `${vaultA} ${vaultB}`);

  app = await Session.start(desktop.webdriver, {
    browserName: "wry",
    "tauri:options": { application: appBinary(), args: [] },
  });
  await waitFor("the setup page", async () => (await path()) === "/_desktop/setup");
  await pairWith(a);
  await waitFor("vault A's note in the local vault", async () => (await localNote(fromA.id)) === 200);
  assert.equal(JSON.parse(readFileSync(join(desktop.dataDir, "device.json"), "utf8")).vault_id, vaultA);
  assert.deepEqual(previous(), []);
  assert.equal(await freshStart(), null);

  // Paired again, with server B: the local copy of A moves aside, whole, and B syncs fresh.
  await pairWith(b);
  await waitFor("vault B's note in the local vault", async () => (await localNote(fromB.id)) === 200);
  assert.equal(JSON.parse(readFileSync(join(desktop.dataDir, "device.json"), "utf8")).vault_id, vaultB);
  const [first] = previous();
  assert.equal(previous().length, 1);
  assert.ok(existsSync(join(first, ".vault/state.db")), "the old local vault is kept whole");
  assert.ok(existsSync(join(first, `notes/note/${fromA.id}.wcl`)), "with vault A's note");
  assert.equal(await localNote(fromA.id), 404, "the fresh local vault holds nothing of A's");
  const fresh = await freshStart();
  assert.equal(fresh.from_vault, vaultA, JSON.stringify(fresh));
  assert.equal(fresh.to_vault, vaultB, JSON.stringify(fresh));
  assert.equal(fresh.previous, first, JSON.stringify(fresh));
  const text = await notice();
  assert.match(text, /joined a different vault/, text);
  assert.ok(text.includes(first.split("/").pop()), text);
  const onB = await b.api("GET", "/api/notes?type=note");
  assert.ok(!JSON.stringify(onB).includes("Kept by vault A"), `B got nothing of A's: ${JSON.stringify(onB)}`);

  // Under the same pairing, B's vault changes: the next round's handshake starts fresh again.
  await b.stop();
  const vaultC = "c".repeat(32);
  execFileSync("sqlite3", [join(b.vault, ".vault/state.db"), `UPDATE meta SET value = '${vaultC}' WHERE key = 'vault_id'`]);
  await b.start();
  assert.equal(await vaultId(b), vaultC);
  const later = await note(b, "Written after the vault changed");
  await app.run("return fetch('/_desktop/sync', { method: 'POST' }).then((r) => r.status);");
  await waitFor("the second fresh start", async () => (await freshStart())?.to_vault === vaultC, 60_000);
  await waitFor("the changed vault's notes in the local vault", async () => (await localNote(later.id)) === 200, 60_000);
  assert.equal(await localNote(fromB.id), 200, "the changed vault is taken whole");
  assert.equal(previous().length, 2);
  const second = previous()[1];
  const again = await freshStart();
  assert.equal(again.from_vault, vaultB, JSON.stringify(again));
  assert.equal(again.previous, second, JSON.stringify(again));
  assert.ok(existsSync(join(second, `notes/note/${fromB.id}.wcl`)), "the copy of B is kept");
  assert.ok((await notice()).includes(second.split("/").pop()));
  const status = await app.run("return fetch('/_desktop/status').then((r) => r.json());");
  assert.equal(status.phase, "synced", JSON.stringify(status));
}
