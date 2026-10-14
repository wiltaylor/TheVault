import type { Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Settings > Scheduled: a scheduled script made, previewed, run and checked in the UI (D10).

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

/** Replace everything in the script editor with `source`, as one input. */
async function replaceCode(page: Page, source: string) {
  await page.locator(".scheduled-script .cm-content").click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText(source);
}

test("a scheduled script made from a preset previews its runs, runs now and creates a note", async ({
  page,
  admin,
}) => {
  const t = token();
  const name = `digest_${t}`;
  const title = `Made by ${name}`;

  await logIn(page, "/settings");
  await page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Scheduled" }).click();
  await expect(page).toHaveURL(/\/settings\/scheduled$/);
  await page.getByRole("link", { name: "New scheduled script" }).click();
  await expect(page).toHaveURL(/\/settings\/scheduled\/new$/);

  await page.getByLabel("Name", { exact: true }).fill(name);
  await page.getByLabel("Label").fill(`Digest ${t}`);
  await page.getByLabel("Repeat").selectOption("weekdays");
  await page.getByLabel("Time of day").fill("07:30");
  await expect(page.getByLabel("Cron schedule")).toHaveValue("30 7 * * 1-5");
  await page.getByLabel("Time zone").fill("Australia/Melbourne");
  await expect(page.getByTestId("schedule-text")).toHaveText("Weekdays at 07:30 (Australia/Melbourne)");
  const next = page.getByRole("list", { name: "Next runs" }).getByRole("listitem");
  await expect(next).toHaveCount(5);
  // Only weekdays.
  for (const item of await next.allTextContents()) expect(item).not.toMatch(/Sat|Sun/);

  await replaceCode(
    page,
    `use vault

fn run() -> Result[string, string] {
    let made = vault::create("task", Value::Map(#{
        "fields": Value::Map(#{ "title": Value::String("${title}") }),
    }))?
    vault::log("created " + made.get("id").unwrap().as_string().unwrap())
    Ok("done")
}
`,
  );
  await page.getByRole("radio", { name: "Any web address" }).check();
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/settings/scheduled/${name}$`));
  const file = await (await admin.get(`/api/tasks/${name}`, { headers: { Accept: "text/plain" } })).text();
  expect(file).toContain(`schedule      = "30 7 * * 1-5"`);
  expect(file).toContain(`allow_domains = ["*"]`);

  await page.getByRole("button", { name: "Run now" }).click();
  await expect(page.locator(".save-bar").getByRole("status")).toHaveText("The run succeeded.");
  const runs = page.getByRole("list", { name: "Runs" });
  await expect(runs.locator("[data-outcome=succeeded]")).toHaveCount(1);
  await expect(runs.getByText(/created [0-9A-Z]{26}/)).toBeVisible();

  const found = await admin.get(`/api/query?q=${encodeURIComponent(`from task where title == "${title}"`)}`);
  const rows = ((await found.json()) as { rows: { id: string }[] }).rows;
  expect(rows).toHaveLength(1);
  const history = (await (await admin.get(`/api/notes/${rows[0]!.id}/history`)).json()) as {
    versions: { author: { kind: string; name: string } }[];
  };
  expect(history.versions[0]!.author).toEqual({ kind: "task", name });

  // The list says it in words, with the last run.
  await page.getByRole("link", { name: "Scheduled", exact: true }).first().click();
  const row = page.locator(`tr[data-task="${name}"]`);
  await expect(row.getByTestId("schedule-text")).toHaveText("Weekdays at 07:30 (Australia/Melbourne)");
  await expect(row.locator("[data-outcome=succeeded]")).toBeVisible();
  await row.getByRole("button", { name: "Run now" }).click();
  await expect(row.getByRole("status")).toHaveText("Succeeded");
});

test("an invalid cron schedule shows an error, and saving it is refused at the field", async ({ page }) => {
  const name = `bad_${token()}`;
  await logIn(page, "/settings/scheduled/new");
  await page.getByLabel("Name", { exact: true }).fill(name);
  await page.getByLabel("Cron schedule").fill("61 * * * *");
  await expect(page.getByTestId("schedule-error")).toContainText("not a valid cron expression");
  await expect(page.getByRole("list", { name: "Next runs" })).toHaveCount(0);
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.locator('[data-issue="schedule"]')).toContainText("not a valid cron expression");
  await expect(page).toHaveURL(/\/settings\/scheduled\/new$/);

  // Fixing it clears the preview error.
  await page.getByLabel("Cron schedule").fill("*/15 * * * *");
  await expect(page.getByTestId("schedule-text")).toHaveText(/^Every 15 minutes \(/);
});

test("old task links still open the scheduled section", async ({ page }) => {
  await logIn(page, "/settings/tasks");
  await expect(page).toHaveURL(/\/settings\/scheduled$/);
  await expect(page.getByRole("heading", { level: 1, name: "Scheduled" })).toBeVisible();
  await page.goto("/settings/tasks/new");
  await expect(page).toHaveURL(/\/settings\/scheduled\/new$/);
  await page.goto("/tasks");
  await expect(page).toHaveURL(/\/settings\/scheduled$/);
});
