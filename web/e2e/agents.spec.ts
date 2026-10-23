import type { Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Agents in the left sidebar and the agent screen (agents G3, G5): create an agent from the
// sidebar's +, set its provider, model and system prompt on the Settings tab with one Save, and
// see it listed in the sidebar and stored in its file. OpenAI is not set up in the test server,
// so its model list cannot be read and the model is typed, as Settings > AI's fallback does.

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
  await expect(pane.getByRole("heading", { name: label })).toBeVisible();
  await expect(agentsNav(page).getByRole("link", { name: label })).toBeVisible();

  const settings = pane.getByRole("region", { name: "Settings" });
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

  // After a reload the sidebar still lists it, and opening it shows what was saved.
  await page.reload();
  await expect(agentsNav(page).getByRole("link", { name: label })).toBeVisible();
  await agentsNav(page).getByRole("link", { name: label }).click();
  const again = activePane(page).getByRole("region", { name: "Settings" });
  await expect(again.getByLabel("Model")).toHaveValue("gpt-e2e");
  await expect(again.getByLabel("System prompt")).toHaveValue("You sort the inbox. Be brief.");

  // The File tab shows the same agent as WCL.
  await activePane(page).getByRole("tab", { name: "File" }).click();
  await expect(activePane(page).getByLabel("Agent file")).toHaveValue(/model\s*=\s*"gpt-e2e"/);

  const del = await admin.delete(`/api/agents/${name}`);
  expect(del.ok(), await del.text()).toBe(true);
});
