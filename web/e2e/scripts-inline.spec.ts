import { expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

test("a scheduled task's script is edited from the task, not a scripts page", async ({ page, admin }) => {
  const t = token();
  const script = `nightly_${t}`;
  const first = `use vault\nfn run() -> Result[int, string] {\n    Ok(0)\n}\n`;
  expect((await admin.put(`/api/scripts/${script}`, { data: { source: first } })).status()).toBe(200);

  // A script made after the app loaded, by an agent say, is offered too.
  await logIn(page, "/settings/tasks/new");
  const field = page.getByLabel("Script", { exact: true });
  await field.selectOption(script);
  await page.getByRole("button", { name: "Edit script" }).click();
  const panel = page.getByRole("group", { name: `Script ${script}` });
  await expect(panel.locator(".cm-content")).toContainText("Ok(0)");

  await panel.locator(".cm-content").click();
  await page.keyboard.press("ControlOrMeta+A");
  const second = `use vault\nfn run() -> Result[int, string] {\n    Ok(1)\n}\n`;
  await page.keyboard.insertText(second);
  await panel.getByRole("button", { name: "Save script" }).click();
  await expect(panel.getByRole("status")).toHaveText("Saved.");
  const stored = (await (await admin.get(`/api/scripts/${script}`)).json()) as { source: string };
  expect(stored.source).toBe(second);

  await page.getByRole("button", { name: "Close script" }).click();
  await expect(panel).toHaveCount(0);
});
