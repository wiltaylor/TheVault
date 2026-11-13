import { expect, logIn, test } from "./fixtures";

test("Dispatched requires a counted preview and explicit current-set confirmation", async ({ page }) => {
  let confirmations = 0;
  await page.route("**/api/dispatched*", (route) => route.fulfill({ json: { threads: [{
    id: "batch-thread", agent: "assistant", title: "Batch review", origin: "trigger:test",
    state: "needs_you", pending: 2, notes: [], question: null, running: false,
    turn: null, started_ms: null, last_activity: null,
  }] } }));
  await page.route("**/api/agents/assistant/threads/batch-thread", (route) => route.fulfill({ json: {
    proposals: [{ event: "proposal", status: "pending", data: { id: "p1", op: "create", type: "memo" } }],
  } }));
  await page.route("**/api/ai/apply-kind/preview", (route) => {
    expect(route.request().postDataJSON()).toEqual({ thread: "batch-thread", proposal: "p1" });
    return route.fulfill({ json: { preview: "fixed-preview", count: 2, kind: ["proposal", "create", "memo"],
      items: [{ id: "p1", summary: "First memo", data: { title: "First" } },
        { id: "p2", summary: "Second memo", data: { title: "Second" } }],
    } });
  });
  await page.route("**/api/ai/apply-kind/confirm", (route) => {
    confirmations += 1;
    expect(route.request().postDataJSON()).toEqual({ preview: "fixed-preview", confirmed: true });
    return route.fulfill({ json: { results: [{ id: "p1", ok: true }, { id: "p2", ok: false, error: { message: "Proposal is stale" } }] } });
  });
  await logIn(page);
  await page.goto("/dispatched");
  await page.getByRole("button", { name: "Review by kind" }).click();
  await page.getByRole("button", { name: /Preview this kind/ }).click();
  const preview = page.getByRole("region", { name: "Apply this kind preview" });
  await expect(preview).toContainText("2 proposals");
  await expect(preview).toContainText("Future proposals still need review");
  await expect(preview.locator("code")).toHaveText(["p1", "p2"]);
  expect(confirmations).toBe(0);
  await preview.getByRole("button", { name: "Confirm Apply 2 proposals" }).click();
  await expect(preview).toContainText("p1: Applied");
  await expect(preview).toContainText("p2: Proposal is stale");
  await preview.getByRole("button", { name: "Retry unsuccessful proposals" }).click();
  await expect.poll(() => confirmations).toBe(2);
});
