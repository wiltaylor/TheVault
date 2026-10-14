import { expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

test("a scheduled script's source is edited on its own page, not a scripts page", async ({ page, admin }) => {
  const t = token();
  const script = `nightly_${t}`;
  const first = `use vault\nfn run() -> Result[int, string] {\n    Ok(0)\n}\n`;
  expect((await admin.put(`/api/scripts/${script}`, { data: { source: first } })).status()).toBe(200);
  // A task made by an agent, naming a script of its own.
  const task = `task_${t}`;
  const put = await admin.put(`/api/tasks/${task}`, { data: { wcl: `schedule = "0 3 * * *"\nscript = "${script}"\n` } });
  expect(put.status(), await put.text()).toBe(200);

  await logIn(page, `/settings/scheduled/${task}`);
  const editor = page.locator(".scheduled-script .cm-content");
  await expect(editor).toContainText("Ok(0)");

  await editor.click();
  await page.keyboard.press("ControlOrMeta+A");
  const second = `use vault\nfn run() -> Result[int, string] {\n    Ok(1)\n}\n`;
  await page.keyboard.insertText(second);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".save-bar").getByRole("status")).toHaveText("Saved.");
  const stored = (await (await admin.get(`/api/scripts/${script}`)).json()) as { source: string };
  expect(stored.source).toBe(second);
  // The task still names the same script.
  const file = await (await admin.get(`/api/tasks/${task}`, { headers: { Accept: "text/plain" } })).text();
  expect(file).toContain(`script   = "${script}"`);
});
