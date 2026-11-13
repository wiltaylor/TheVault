const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

test("desktop pages have no native IPC permissions, including menu and tray writes", () => {
  const root = path.join(__dirname, "..");
  const config = JSON.parse(fs.readFileSync(path.join(root, "tauri.conf.json")));
  assert.deepEqual(config.app.security.capabilities ?? [], []);
  const capabilities = fs.readdirSync(path.join(root, "capabilities"));
  assert.deepEqual(capabilities, ["default.json"]);
  for (const file of capabilities) {
    const capability = JSON.parse(fs.readFileSync(path.join(root, "capabilities", file)));
    assert.deepEqual(capability.permissions, []);
  }
});
