import type { APIResponse } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// A query view's query is written in the query language; there is no visual builder.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function ok(res: Promise<APIResponse>, want: number) {
  const r = await res;
  expect(r.status(), await r.text()).toBe(want);
  return r;
}

test("a query view's query is typed as text, and a syntax error shows without saving over it", async ({ page, admin }) => {
  const title = `Typed ${token()}`;
  const blocks = [
    {
      kind: "content",
      labels: ["body"],
      blocks: [{ kind: "query_view", fields: { query: "from *" }, blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table" } }] }],
    },
  ];
  const res = await ok(admin.post("/api/notes?type=page", { data: { note: { fields: { title }, blocks } } }), 201);
  const { id } = (await res.json()) as { id: string };
  await logIn(page, `/notes/${id}`);
  await page.getByRole("button", { name: "Edit query view" }).click();
  const settings = page.getByRole("group", { name: "Query view settings" });
  const query = settings.getByRole("textbox", { name: "Query", exact: true });
  await expect(query).toHaveValue("from *");
  await expect(settings.getByRole("button", { name: "Builder" })).toHaveCount(0);

  await query.fill("from task where");
  await expect(settings.getByRole("alert")).toBeVisible();
  await expect(query).toHaveAttribute("aria-invalid", "true");

  await query.fill(`from task, page search "${title}"`);
  await expect(settings.getByRole("alert")).toHaveCount(0);
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  expect(((await (await admin.get(`/api/notes/${id}`)).json()) as { wcl: string }).wcl).toContain(`from task, page search`);
});
