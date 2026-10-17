// The release smoke test (D11): the AppImage `just desktop-release` built starts under Xvfb and
// shows the setup page. It runs the AppImage itself through tauri-driver, extracted rather than
// mounted, since the container has no FUSE. Run it with `just desktop-release-test`.

import assert from "node:assert/strict";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { Desktop, removeRoot, repo, tempRoot } from "./harness.mjs";
import { Session, waitFor } from "./webdriver.mjs";

function appImage() {
  if (process.env.THEVAULT_APPIMAGE) return process.env.THEVAULT_APPIMAGE;
  const dir = join(repo, "apps/desktop/src-tauri/target/release/bundle/appimage");
  const found = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".AppImage")) : [];
  if (found.length !== 1) throw new Error(`expected one AppImage in ${dir}, found ${found.length}; run \`just desktop-release\`, or set THEVAULT_APPIMAGE.`);
  return join(dir, found[0]);
}

let root;
let desktop;
let app;

before(async () => {
  root = tempRoot();
  desktop = new Desktop(root);
  desktop.env.APPIMAGE_EXTRACT_AND_RUN = "1";
  desktop.env.TMPDIR = join(root, "runtime");
  // Installing itself in ~/Applications would start a second copy and end the one tauri-driver drives.
  desktop.env.THEVAULT_NO_SELF_INSTALL = "1";
  await desktop.start();
});

after(async () => {
  await app?.quit().catch(() => {});
  await desktop?.stop();
  if (root) removeRoot(root);
});

test("the AppImage starts and shows the setup page", async () => {
  app = await Session.start(desktop.webdriver, {
    browserName: "wry",
    "tauri:options": { application: appImage() },
  });
  await waitFor("the setup page", async () => new URL(await app.url()).pathname === "/_desktop/setup", 120_000);
  assert.equal(await app.text("h1"), "Set up TheVault");
});
