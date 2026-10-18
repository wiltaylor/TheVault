import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, logIn, test } from "./fixtures";

// A template tab's `when` (D14, D42): the tab shows only while the note meets the condition,
// and appears or disappears as the field is edited, before the note is saved.

const repo = fileURLToPath(new URL("../..", import.meta.url));

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE_WCL = `label = "Draft"
plural = "Drafts"

field stage {
  kind    = "select"
  label   = "Stage"
  options = ["idea", "draft", "final"]
}

field reviewer {
  kind  = "string"
  label = "Reviewer"
}

template default {
  tab "Main" {
    slot "title"
    slot "stage"
  }

  tab "Notes" {
    paragraph {
      text = "Notes on {{title}}."
    }
  }

  tab "Review" {
    when = "stage == \\"draft\\" || stage == \\"final\\""
    paragraph {
      text = "Ready for review."
    }
    slot "reviewer"
  }
}
`;

test("a conditional tab shows and hides as the field it reads is edited", async ({ page, admin }) => {
  const name = `when_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  const schema = (await (await admin.get("/api/schema")).json()) as {
    types: { name: string; templates: { tabs: { title: string; when?: string }[] }[] }[];
  };
  const tabs = schema.types.find((t) => t.name === name)!.templates[0]!.tabs;
  expect(tabs.map((t) => t.when ?? null)).toEqual([null, null, 'stage == "draft" || stage == "final"']);

  const res = await admin.post(`/api/notes?type=${name}`, {
    data: { note: { fields: { title: "Field guide", stage: "idea" }, blocks: [] } },
  });
  expect(res.status(), await res.text()).toBe(201);
  const id = ((await res.json()) as { id: string }).id;

  await logIn(page, `/notes/${id}`);
  const tablist = page.locator(".note-main").getByRole("tablist");
  await expect(tablist.getByRole("tab")).toHaveText(["Main", "Notes"]);

  // Editing the field shows the tab at once.
  const stage = page.getByLabel("Stage");
  await stage.selectOption("draft");
  await expect(tablist.getByRole("tab")).toHaveText(["Main", "Notes", "Review"]);
  await tablist.getByRole("tab", { name: "Review" }).click();
  await expect(page.getByText("Ready for review.")).toBeVisible();
  await expect(page.getByLabel("Reviewer")).toBeVisible();
  const shots = join(repo, ".scratch", "shots");
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, "ticket-when-tab.png") });

  // Back to an idea: the open tab is hidden, and the page moves to the first tab.
  await tablist.getByRole("tab", { name: "Main" }).click();
  await stage.selectOption("idea");
  await expect(tablist.getByRole("tab")).toHaveText(["Main", "Notes"]);

  // Saved as final, the tab is there when the note is opened again.
  await stage.selectOption("final");
  await expect(tablist.getByRole("tab")).toHaveText(["Main", "Notes", "Review"]);
  await expect
    .poll(async () => ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: { stage?: string } } }).note.fields.stage)
    .toBe("final");
  await page.reload();
  await expect(page.locator(".note-main").getByRole("tablist").getByRole("tab")).toHaveText(["Main", "Notes", "Review"]);
});

test("the template editor edits a tab's when and shows a bad condition inline", async ({ page, admin }) => {
  const name = `when_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);

  await logIn(page, `/settings/types/${name}/templates`);
  await page.getByRole("tab", { name: "Review" }).click();
  const when = page.getByLabel("Show this tab when");
  await expect(when).toHaveValue('stage == "draft" || stage == "final"');

  await when.fill("stge == \"draft\"");
  await expect(page.getByRole("alert")).toHaveText("`stge` is not a field of the type");
  await when.fill("stage ==");
  await expect(page.getByRole("alert")).toContainText("does not parse: 1:9:");

  await when.fill('stage == "final"');
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  const tpl = (await (await admin.get(`/api/types/${name}/templates/default`)).json()) as { wcl: string };
  expect(tpl.wcl).toContain('when = "stage == \\"final\\""');

  // A tab cleared of its condition shows on every note.
  await when.fill("");
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  const cleared = (await (await admin.get(`/api/types/${name}/templates/default`)).json()) as { wcl: string };
  expect(cleared.wcl).not.toContain("when");
});
