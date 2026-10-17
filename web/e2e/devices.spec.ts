import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { type APIRequestContext, request } from "@playwright/test";
import { expect, logIn, password, test } from "./fixtures";

// Settings > Devices: a paired device shows with its settings; rename, the switches, enabled
// commands, the exclusion list and the skip pattern save; revoke ends the device's token; "Pair a
// device" shows a QR code with a code an app pairs with once (S2; D2, D3, D25).

function unique(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

interface Paired {
  device_id: string;
  token: string;
}

/** Pair a device the way an app does, with no credentials but the password or a code. */
async function pair(body: Record<string, unknown>): Promise<{ status: number; body: Paired }> {
  const ctx = await request.newContext({ baseURL: process.env.E2E_BASE_URL });
  const res = await ctx.post("/api/devices/pair", { data: body });
  const out = { status: res.status(), body: (await res.json()) as Paired };
  await ctx.dispose();
  return out;
}

async function deviceStatus(token: string): Promise<number> {
  const ctx = await request.newContext({
    baseURL: process.env.E2E_BASE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${token}` },
  });
  const status = (await ctx.get("/api/devices")).status();
  await ctx.dispose();
  return status;
}

async function deviceById(admin: APIRequestContext, id: string) {
  const res = await admin.get("/api/devices");
  expect(res.status()).toBe(200);
  const { devices } = (await res.json()) as { devices: { id: string; name: string; settings: Record<string, unknown> }[] };
  return devices.find((d) => d.id === id);
}

test("a paired device can be renamed, set up and revoked", async ({ page, admin }) => {
  const name = `Pixel ${unique()}`;
  const paired = await pair({ password: password(), name, platform: "android", capabilities: ["sms"] });
  expect(paired.status).toBe(201);
  const id = paired.body.device_id;

  await logIn(page, "/settings/devices");
  await expect(page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Devices" })).toBeVisible();
  const card = page.locator(`[data-device="${id}"]`);
  await expect(card.getByRole("heading", { name })).toBeVisible();
  await expect(card).toContainText("Android");

  const renamed = `Work phone ${unique()}`;
  await card.getByLabel("Name").fill(renamed);
  await card.getByLabel("Send SMS").check();
  await card.getByRole("group", { name: "Enabled commands" }).getByLabel("send_sms").check();
  await card.getByRole("button", { name: "Add exclusion" }).click();
  await card.getByLabel("Excluded value").fill("+61400000000");
  await card.getByRole("button", { name: "Add exclusion" }).click();
  await card.getByLabel("Exclusion kind").nth(1).selectOption("thread");
  await card.getByLabel("Excluded value").nth(1).fill("42");
  await card.getByLabel("Skip pattern").fill("(?i)code");
  await card.getByRole("button", { name: "Save" }).click();
  await expect(card.getByRole("status")).toHaveText("Saved.");
  await expect(card.getByRole("heading", { name: renamed })).toBeVisible();

  const saved = await deviceById(admin, id);
  expect(saved?.name).toBe(renamed);
  expect(saved?.settings).toEqual({
    commands: ["send_sms"],
    contacts_write: false,
    sms_send: true,
    exclusions: [
      { kind: "number", value: "+61400000000" },
      { kind: "thread", value: "42" },
    ],
    skip_pattern: "(?i)code",
  });

  // A bad skip pattern is refused with the server's reason.
  await card.getByLabel("Skip pattern").fill("(");
  await card.getByRole("button", { name: "Save" }).click();
  await expect(card.getByRole("alert")).toContainText("skip pattern");

  // What was saved shows again after a reload.
  await page.reload();
  const again = page.locator(`[data-device="${id}"]`);
  await expect(again.getByLabel("Name")).toHaveValue(renamed);
  await expect(again.getByLabel("Send SMS")).toBeChecked();
  await expect(again.getByLabel("Write phone contacts")).not.toBeChecked();
  await expect(again.getByLabel("Excluded value").nth(1)).toHaveValue("42");
  await expect(again.getByLabel("Skip pattern")).toHaveValue("(?i)code");

  // Remove one exclusion and save.
  await again.getByRole("button", { name: "Remove exclusion" }).first().click();
  await again.getByRole("button", { name: "Save" }).click();
  await expect(again.getByRole("status")).toHaveText("Saved.");
  expect((await deviceById(admin, id))?.settings.exclusions).toEqual([{ kind: "thread", value: "42" }]);

  expect(await deviceStatus(paired.body.token)).toBe(200);
  page.once("dialog", (d) => void d.accept());
  await again.getByRole("button", { name: "Revoke" }).click();
  await expect(page.locator(`[data-device="${id}"]`)).toHaveCount(0);
  expect(await deviceById(admin, id)).toBeUndefined();
  expect(await deviceStatus(paired.body.token)).toBe(401);
});

test("pair a device shows a QR code and a code that pairs once", async ({ page, baseURL }) => {
  await logIn(page, "/settings/devices");
  await page.getByRole("button", { name: "Pair a device" }).click();
  const panel = page.getByRole("region", { name: "Pair a device" });
  const qr = panel.getByRole("img", { name: "Pairing QR code" });
  await expect(qr).toBeVisible();
  await expect(qr).toHaveAttribute("src", /^data:image\/svg\+xml/);
  // The image decoded: an SVG that failed to load has no natural size.
  expect(await qr.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
  const code = (await panel.getByLabel("Pairing code").textContent())!.trim();
  expect(code).toMatch(/^[0-9A-Z]{4}-[0-9A-Z]{4}$/);
  await expect(panel).toContainText(new URL(baseURL!).origin);

  const name = `Laptop ${unique()}`;
  const first = await pair({ code, name, platform: "linux" });
  expect(first.status).toBe(201);
  expect((await pair({ code, name: `${name} again`, platform: "linux" })).status).toBe(401);

  await panel.getByRole("button", { name: "Done" }).click();
  await expect(panel).toHaveCount(0);
  const card = page.locator(`[data-device="${first.body.device_id}"]`);
  await expect(card.getByRole("heading", { name })).toBeVisible();
  await expect(card).toContainText("Linux desktop");
});

/** The default `device_import` script, as the starter vault holds it. */
const DEVICE_IMPORT = readFileSync(
  fileURLToPath(new URL("../../crates/writer/defaults/devices/device_import.ws", import.meta.url)),
  "utf8",
);

test("Import from phone queues import_history with the chosen options once it is enabled", async ({ page, admin }) => {
  // The vault's Android device file declares the command, with the default script.
  expect((await admin.put("/api/scripts/device_import", { data: { source: DEVICE_IMPORT } })).status()).toBe(200);
  const file = await admin.put("/api/device-files/android", {
    data: { wcl: 'command "import_history" {\n  script = "device_import"\n}\n' },
  });
  expect(file.status(), await file.text()).toBe(200);
  const name = `Pixel ${unique()}`;
  const paired = await pair({ password: password(), name, platform: "android", capabilities: ["sms"] });
  expect(paired.status).toBe(201);
  const id = paired.body.device_id;

  await logIn(page, "/settings/devices");
  const card = page.locator(`[data-device="${id}"]`);
  const importer = card.getByRole("group", { name: "Import from phone" });
  const button = importer.getByRole("button", { name: "Import from phone" });

  // Declared, but not enabled for this phone: the button explains instead.
  await expect(button).toBeDisabled();
  await expect(importer).toContainText("Turn on import_history under Enabled commands above and save");
  await expect(importer).toContainText("RCS chats cannot be imported");

  await card.getByRole("group", { name: "Enabled commands" }).getByLabel("import_history").check();
  await card.getByRole("button", { name: "Save" }).click();
  await expect(card.getByRole("status").first()).toHaveText("Saved.");
  await expect(button).toBeEnabled();

  // SMS and MMS and contacts, over September.
  await importer.getByLabel("Calls", { exact: true }).uncheck();
  await importer.getByLabel("Contacts", { exact: true }).check();
  await importer.getByLabel("Dates").selectOption("custom");
  await importer.getByLabel("From", { exact: true }).fill("2026-09-01");
  await importer.getByLabel("To", { exact: true }).fill("2026-09-30");
  await button.click();
  const progress = importer.getByRole("status", { name: "Import progress" });
  await expect(progress).toContainText("Waiting for the phone");

  const listed = await admin.get(`/api/devices/${id}/commands`);
  const { commands } = (await listed.json()) as { commands: { id: string; name: string; status: string; payload: Record<string, unknown> }[] };
  expect(commands).toHaveLength(1);
  const command = commands[0]!;
  expect(command.name).toBe("import_history");
  expect(command.status).toBe("pending");
  const [since, until] = await page.evaluate(() => [
    new Date(2026, 8, 1).toISOString(),
    new Date(new Date(2026, 9, 1).getTime() - 1).toISOString(),
  ]);
  expect(command.payload).toEqual({ kinds: ["sms", "mms", "contacts"], since, until });

  // The phone reports its progress, then its result, and the card follows.
  const phone = await request.newContext({
    baseURL: process.env.E2E_BASE_URL,
    extraHTTPHeaders: { Authorization: `Bearer ${paired.body.token}` },
  });
  const report = (body: unknown) => phone.post(`/api/devices/${id}/commands/${command.id}/result`, { data: body });
  const sms = { kind: "sms", found: 120, imported: 3, notes: 3, existing: 117, skipped: 0, failed: 0, missing_permission: null, error: null };
  expect((await report({ status: "delivered" })).status()).toBe(200);
  const running = { kinds: [{ ...sms, done: false }], since: null, until: null, done: false };
  expect((await report({ status: "delivered", result: { progress: running } })).status()).toBe(200);
  await expect(progress).toContainText("The phone is importing");
  await expect(progress).toContainText("SMS: 120 found, 3 new notes, 117 already in the vault…");

  const contacts = { ...sms, kind: "contacts", found: 0, imported: 0, notes: 0, existing: 0, missing_permission: "android.permission.READ_CONTACTS" };
  const result = { kinds: [{ ...sms, done: true }, { ...contacts, done: true }], since: null, until: null, done: true, message: "" };
  expect((await report({ status: "done", result })).status()).toBe(200);
  await expect(progress).toContainText("Import finished.");
  await expect(progress).toContainText("Contacts: not read. Android has not given TheVault READ_CONTACTS.");
  await expect(progress).toContainText("Open TheVault on the phone and allow it");
  await phone.dispose();

  // Without the command enabled again, the button says so once more.
  await card.getByRole("group", { name: "Enabled commands" }).getByLabel("import_history").uncheck();
  await card.getByRole("button", { name: "Save" }).click();
  await expect(button).toBeDisabled();
});

test("in an app, only the app's own phone can be asked to import", async ({ page, admin }) => {
  expect((await admin.put("/api/scripts/device_import", { data: { source: DEVICE_IMPORT } })).status()).toBe(200);
  const file = await admin.put("/api/device-files/android", {
    data: { wcl: 'command "import_history" {\n  script = "device_import"\n}\n' },
  });
  expect(file.status(), await file.text()).toBe(200);
  const own = await pair({ password: password(), name: `Own ${unique()}`, platform: "android" });
  const other = await pair({ password: password(), name: `Other ${unique()}`, platform: "android" });
  for (const d of [own, other]) {
    const res = await admin.patch(`/api/devices/${d.body.device_id}`, { data: { settings: { commands: ["import_history"] } } });
    expect(res.status()).toBe(200);
  }
  // The page runs in the app paired as `own`, as its embedded server says.
  await page.route("**/api/app", (route) =>
    route.fulfill({ json: { app: true, paired: true, device_id: own.body.device_id, device_name: "Own", server_url: "https://home" } }),
  );
  await logIn(page, "/settings/devices");
  const importer = (d: { body: Paired }) =>
    page.locator(`[data-device="${d.body.device_id}"]`).getByRole("group", { name: "Import from phone" });
  await expect(importer(other).getByRole("button", { name: "Import from phone" })).toBeDisabled();
  await expect(importer(other)).toContainText("Ask another phone to import from the web app");
  // The app's own phone, declared and enabled, can be asked.
  await expect(importer(own).getByRole("button", { name: "Import from phone" })).toBeEnabled();
});
