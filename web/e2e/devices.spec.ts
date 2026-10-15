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
