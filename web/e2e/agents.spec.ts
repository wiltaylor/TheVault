import type { Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Agents in the left sidebar and the agent screen (agents G3, G5): create an agent from the
// sidebar's +, which opens on its chat; open its settings with the cog, set its provider, model
// and system prompt on the General tab with one Save, and see it listed in the sidebar and stored
// in its file; the old tab addresses lead to the settings tabs. OpenAI is not set up in the test
// server, so its model list cannot be read and the model is typed, as Settings > AI's fallback
// does. On a phone the chat's bar fits the screen.

function agentsNav(page: Page) {
  return page.getByRole("navigation", { name: "Agents" });
}

test("an agent created from the sidebar takes a model and prompt and is listed", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const label = `Agent ${stamp}`;
  const name = `agent_${stamp}`;

  await logIn(page, "/daily");
  // Assistant is built in and listed first.
  await expect(agentsNav(page).getByRole("link").first()).toHaveText("Assistant");

  await agentsNav(page).getByRole("button", { name: "New agent" }).click();
  const box = agentsNav(page).getByLabel("New agent name");
  await box.fill(label);
  await box.press("Enter");

  await expect(page).toHaveURL(new RegExp(`/agents/${name}$`));
  const pane = activePane(page);
  await expect(agentsNav(page).getByRole("link", { name: label })).toBeVisible();

  // The screen opens on the chat; the model and prompt are in the settings behind the cog.
  const chat = pane.getByRole("region", { name: "Chat" });
  await expect(chat.getByRole("button", { name: "Chat session: New chat" })).toBeVisible();
  await expect(chat.getByLabel("Ask AI")).toBeVisible();
  await chat.getByRole("link", { name: `Settings of ${label}` }).click();
  await expect(page).toHaveURL(new RegExp(`/agents/${name}/settings$`));
  await expect(pane.getByRole("heading", { name: label })).toBeVisible();
  await expect(pane.getByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true");
  const settings = pane.getByRole("region", { name: "General" });
  await settings.getByLabel("Provider").selectOption("openai");
  const model = settings.getByLabel("Model");
  await expect(model).toHaveAttribute("placeholder", "Provider default");
  await model.fill("gpt-e2e");
  await model.press("Tab");
  const prompt = settings.getByLabel("System prompt");
  await expect(prompt).toHaveAttribute("placeholder", /.+/);
  await prompt.fill("You sort the inbox. Be brief.");
  await settings.getByRole("button", { name: "Save" }).click();
  await expect(settings.getByRole("status")).toHaveText("Saved.");

  const res = await admin.get(`/api/agents/${name}`);
  expect(res.status(), await res.text()).toBe(200);
  const saved = (await res.json()) as { label: string; provider: string; model: string; system_prompt: string };
  expect(saved).toMatchObject({ label, provider: "openai", model: "gpt-e2e", system_prompt: "You sort the inbox. Be brief." });

  // After a reload the sidebar still lists it, and its settings show what was saved.
  await page.reload();
  await expect(agentsNav(page).getByRole("link", { name: label })).toBeVisible();
  const again = activePane(page).getByRole("region", { name: "General" });
  await expect(again.getByLabel("Model")).toHaveValue("gpt-e2e");
  await expect(again.getByLabel("System prompt")).toHaveValue("You sort the inbox. Be brief.");

  // The File tab shows the same agent as WCL.
  await activePane(page).getByRole("tab", { name: "File" }).click();
  await expect(page).toHaveURL(new RegExp(`/agents/${name}/settings/file$`));
  await expect(activePane(page).getByLabel("Agent file")).toHaveValue(/model\s*=\s*"gpt-e2e"/);

  // Back to the chat; the sidebar's link opens the chat too.
  await activePane(page).getByRole("link", { name: "Back to the chat" }).click();
  await expect(page).toHaveURL(new RegExp(`/agents/${name}$`));
  await expect(activePane(page).getByRole("region", { name: "Chat" })).toBeVisible();

  const del = await admin.delete(`/api/agents/${name}`);
  expect(del.ok(), await del.text()).toBe(true);
});

test("the agent screen's old tab addresses lead to the settings tabs", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = `old_tabs_${stamp}`;
  expect((await admin.put(`/api/agents/${name}`, { data: { wcl: `label = "Old tabs ${stamp}"\n` } })).ok()).toBe(true);

  await logIn(page, `/agents/${name}/permissions`);
  await expect(page).toHaveURL(new RegExp(`/agents/${name}/settings/permissions$`));
  await expect(activePane(page).getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
  await page.goto(`/agents/${name}/memory`);
  await expect(page).toHaveURL(new RegExp(`/agents/${name}/settings/memory$`));
  await expect(activePane(page).getByRole("region", { name: "Memory" })).toBeVisible();
  await page.goto(`/agents/${name}/chat`);
  await expect(page).toHaveURL(new RegExp(`/agents/${name}$`));
  await expect(activePane(page).getByRole("region", { name: "Chat" })).toBeVisible();

  expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});

test("on a phone the agent chat's bar wraps within the screen", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = `phone_${stamp}`;
  const label = `A rather long agent label ${stamp}`;
  expect((await admin.put(`/api/agents/${name}`, { data: { wcl: `label = "${label}"\n` } })).ok()).toBe(true);
  const made = await admin.post(`/api/agents/${name}/threads`, { data: { title: `A long chat title that will not fit on a phone ${stamp}` } });
  expect(made.ok(), await made.text()).toBe(true);
  const thread = ((await made.json()) as { id: string }).id;

  await page.setViewportSize({ width: 390, height: 844 });
  await logIn(page, `/agents/${name}?thread=${thread}`);
  const chat = activePane(page).getByRole("region", { name: "Chat" });
  await expect(chat.getByRole("button", { name: /^Chat session: A long chat title/ })).toBeVisible();
  for (const control of [
    chat.getByRole("button", { name: "Archive chat" }),
    chat.getByRole("button", { name: "Delete chat" }),
    chat.getByRole("link", { name: `Settings of ${label}` }),
    chat.getByLabel("Ask AI"),
    chat.getByRole("button", { name: "Send" }),
  ]) {
    await expect(control).toBeVisible();
    const box = (await control.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(390);
  }
  const overflow = await chat.evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  // The picker's list fits the screen too.
  await chat.getByRole("button", { name: /^Chat session: / }).click();
  const list = chat.getByRole("dialog", { name: "Chat sessions" });
  await expect(list).toBeVisible();
  const box = (await list.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(390);

  expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});
