import type { APIRequestContext, APIResponse, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Multi-value fields are picked as chips with a + after them: on a note page and in the
// Properties panel.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function ok(res: Promise<APIResponse>, want: number) {
  const r = await res;
  expect(r.status(), await r.text()).toBe(want);
  return r;
}

/**
 * A type whose template places refs, tags and topics on the page, and leaves side_refs,
 * side_tags and side_topics for the Properties panel, over a taxonomy three levels deep.
 */
async function setup(admin: APIRequestContext) {
  const t = token();
  const tax = `place_${t}`;
  await ok(
    admin.put(`/api/taxonomies/${tax}`, {
      data: { wcl: `label = "Places"\n\nterm "Plant" {\n  term "Electrical" {\n    term "Wiring"\n  }\n}\n\nterm "Office"\n` },
    }),
    200,
  );
  const fields = (prefix: string) =>
    `field ${prefix}refs {\n  kind = "reference"\n  multiple = true\n  target = "task"\n  label = "${prefix ? "Side refs" : "Refs"}"\n}\n` +
    `field ${prefix}tags {\n  kind = "list"\n  items = "string"\n  label = "${prefix ? "Side tags" : "Tags"}"\n}\n` +
    `field ${prefix}topics {\n  kind = "taxonomy"\n  multiple = true\n  taxonomy = "${tax}"\n  label = "${prefix ? "Side topics" : "Topics"}"\n}\n`;
  const name = `chips_${t}`;
  const wcl =
    `label = "Chips"\n\n${fields("")}${fields("side_")}\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "refs"\n    slot "tags"\n    slot "topics"\n  }\n}\n`;
  await ok(admin.put(`/api/types/${name}`, { data: { wcl } }), 200);
  const make = async (type: string, title: string) =>
    ((await (await ok(admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks: [] } } }), 201)).json()) as {
      id: string;
    }).id;
  const first = await make("task", `Chip target ${t} one`);
  const second = await make("task", `Chip target ${t} two`);
  const note = await make(name, `Chips ${t}`);
  return { t, name, note, first, second };
}

async function fields(admin: APIRequestContext, id: string): Promise<Record<string, unknown>> {
  return ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Open the picker's + box if it is not open, type `typed`, and pick the option `option`. */
async function pick(scope: Locator, label: string, typed: string, option: string | RegExp) {
  const box = scope.getByRole("combobox", { name: `Add to ${label}` });
  if (!(await box.isVisible())) await scope.getByRole("button", { name: `Add to ${label}` }).click();
  await expect(box).toBeFocused();
  await box.fill(typed);
  await scope.getByRole("option", { name: option }).click();
}

/** Add, link and remove references, add list items, and add a deep taxonomy term, in `scope`. */
async function exercise(page: Page, scope: (f: string) => Locator, labels: [string, string, string], s: Awaited<ReturnType<typeof setup>>, prefix: string) {
  const [refs, tags, topics] = labels;
  // No box round the chips and no search box until + is pressed.
  await expect(scope(`${prefix}refs`).getByRole("combobox")).toHaveCount(0);
  expect(await scope(`${prefix}refs`).locator(".chip-picker").evaluate((el) => getComputedStyle(el).borderTopStyle)).toBe("none");
  await pick(scope(`${prefix}refs`), refs, s.t, `Chip target ${s.t} one`);
  // The box stays open for the next, and the picked note is not offered again.
  await scope(`${prefix}refs`).getByRole("combobox", { name: `Add to ${refs}` }).fill(s.t);
  await expect(scope(`${prefix}refs`).getByRole("option", { name: `Chip target ${s.t} two` })).toBeVisible();
  await expect(scope(`${prefix}refs`).getByRole("option", { name: `Chip target ${s.t} one` })).toHaveCount(0);
  await scope(`${prefix}refs`).getByRole("option", { name: `Chip target ${s.t} two` }).click();
  await expect(scope(`${prefix}refs`).getByRole("link", { name: `Chip target ${s.t} one` })).toHaveAttribute("href", `/notes/${s.first}`);
  await scope(`${prefix}refs`).getByRole("combobox", { name: `Add to ${refs}` }).press("Escape");
  await expect(scope(`${prefix}refs`).getByRole("button", { name: `Add to ${refs}` })).toBeFocused();
  await scope(`${prefix}refs`).getByRole("button", { name: `Remove Chip target ${s.t} one` }).click();
  await expect(scope(`${prefix}refs`).locator(".chip")).toHaveText([`Chip target ${s.t} two×`]);

  await scope(`${prefix}tags`).getByRole("button", { name: `Add to ${tags}` }).click();
  const tagBox = scope(`${prefix}tags`).getByRole("combobox", { name: `Add to ${tags}` });
  await tagBox.fill("red");
  await tagBox.press("Enter");
  await tagBox.fill("green");
  await tagBox.press("Enter");
  await tagBox.fill("blue");
  await tagBox.press("Enter");
  // Backspace in the empty box takes the last chip off.
  await tagBox.press("Backspace");
  await expect(scope(`${prefix}tags`).locator(".chip")).toHaveText(["red×", "green×"]);

  // A term two levels down, found by typing part of its name; it says what it is under.
  await pick(scope(`${prefix}topics`), topics, "wir", /^Wiring\s*Plant\/Electrical$/);
  await expect(scope(`${prefix}topics`).locator(".chip")).toHaveText(["Plant/Electrical/Wiring×"]);

  await page.getByLabel("Title", { exact: true }).focus();
  await saved(page);
}

test("on a note page, references, list items and deep terms are picked as chips", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.note}`);
  await exercise(page, (f) => page.getByTestId(`field-${f}`), ["Refs", "Tags", "Topics"], s, "");
  expect(await fields(admin, s.note)).toMatchObject({
    refs: [s.second],
    tags: ["red", "green"],
    topics: ["Plant/Electrical/Wiring"],
  });
});

test("in the Properties panel, the same pickers edit the fields the page does not place", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.note}`);
  await page.getByRole("button", { name: "Properties", exact: true }).click();
  const sidebar = page.getByRole("complementary", { name: "Properties" });
  await expect(sidebar).toBeVisible();
  await exercise(page, (f) => sidebar.getByTestId(`field-${f}`), ["Side refs", "Side tags", "Side topics"], s, "side_");
  expect(await fields(admin, s.note)).toMatchObject({
    side_refs: [s.second],
    side_tags: ["red", "green"],
    side_topics: ["Plant/Electrical/Wiring"],
  });
});
