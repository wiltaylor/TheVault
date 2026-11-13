import { expect, logIn, test } from "./fixtures";

test("a delayed ready item shows minutes and Cancel send returns it to Draft", async ({ page, admin }) => {
  // Keep the real item in draft so no provider is called. Model the ready read and
  // server-owned deadline at the HTTP boundary. Server/writer tests cover admission.
  const made = await admin.post("/api/notes?type=outbox", {
    data: { note: { fields: { title: "Delayed email", kind: "email", status: "draft", to: ["ann@example.com"] } } },
  });
  expect(made.ok()).toBe(true);
  const saved = await made.json();
  let cancelled = false;
  const deadline = Date.now() + 300_000;
  await page.route("**/api/query", async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    if (!cancelled && route.request().postData()?.includes('ready')) {
      body.rows = [{ id: saved.id, type: "outbox", title: "Delayed email", fields: { ...saved.note.fields, status: "ready" } }];
      body.total = 1;
    }
    await route.fulfill({ response, json: body });
  });
  await page.route(`**/api/notes/${saved.id}`, async (route) => {
    if (route.request().method() === "PUT") {
      cancelled = true;
      await route.continue();
      return;
    }
    const response = await route.fetch();
    const body = await response.json();
    if (!cancelled) {
      body.note.fields.status = "ready";
      body.send_deadline_ms = deadline;
    }
    await route.fulfill({ response, json: body });
  });
  await logIn(page, "/outbox?tab=ready");
  const item = page.locator(`li[data-note="${saved.id}"]`);
  await expect(item.getByText("Sends in 5 minutes")).toBeVisible();
  await item.getByRole("button", { name: "Cancel send" }).click();
  await expect(item).toHaveCount(0);
  await page.getByRole("tab", { name: "Draft", exact: true }).click();
  await expect(page.getByRole("link", { name: "Delayed email", exact: true })).toBeVisible();
  const stored = await (await admin.get(`/api/notes/${saved.id}`)).json();
  expect(stored.note.fields.status).toBe("draft");
  expect(stored.note.fields.sent_at).toBeUndefined();
  // A read still in flight when the page closes must not fail the finished test (TV-274).
  await page.unrouteAll({ behavior: "ignoreErrors" });
});
