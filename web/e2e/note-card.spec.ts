import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// A `card` content block (docs/vault-format.md, "card"): inserted from the `/` menu, which opens
// a note search; the chosen note shows as a card through its type's `card` template, is stored
// as `card { note = ... }`, links to the note as a mention does, opens the note when clicked, and
// shows as missing once the note is deleted.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE_WCL = `label = "Shelf"
plural = "Shelves"

field blurb {
  kind  = "string"
  label = "Blurb"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "blurb"
    slot "body"
  }
}

template card {
  tab "Card" {
    slot "blurb"
  }
}
`;

type Json = { kind: string; fields?: Record<string, unknown>; blocks?: Json[]; labels?: string[] };

interface Made {
  id: string;
  version: string;
}

async function setup(admin: APIRequestContext) {
  const t = token();
  const type = `shelf_${t}`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  const post = async (fields: Record<string, unknown>): Promise<Made> => {
    const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
    expect(res.status(), await res.text()).toBe(201);
    return (await res.json()) as Made;
  };
  const targetTitle = `Carded target ${t}`;
  const target = await post({ title: targetTitle, blurb: `Blurb of ${t}` });
  const source = await post({ title: `Card holder ${t}` });
  return { t, target, targetTitle, source };
}

async function storedBody(admin: APIRequestContext, id: string): Promise<Json[]> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  const n = (await res.json()) as { note: { blocks: Json[] } };
  return n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks ?? [];
}

const body = (page: Page): Locator => activePane(page).getByRole("textbox", { name: "Body" });

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

test("a note card from the / menu shows the note's card template, links to it and opens it", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.source.id}`);
  await body(page).click();

  await page.keyboard.type("/card");
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "Note card" }).click();
  const search = body(page).getByRole("combobox", { name: "Note to show as a card" });
  await expect(search).toBeFocused();
  await page.keyboard.type(`Carded target ${s.t}`);
  await page.getByRole("option", { name: s.targetTitle }).click();

  const card = body(page).getByTestId("note-card");
  await expect(card.getByTestId("note-card-title")).toHaveText(s.targetTitle);
  await expect(card.locator(".query-slot[data-field='blurb']")).toContainText(`Blurb of ${s.t}`);
  await saved(page);
  await expect.poll(() => storedBody(admin, s.source.id)).toEqual([{ kind: "card", fields: { note: s.target.id } }]);

  // The card is a link: the target's graph has the holder linking to it, as a mention would.
  const graph = await admin.get(`/api/notes/${s.target.id}/graph`);
  expect(graph.status()).toBe(200);
  const g = (await graph.json()) as { edges: { from: string; to: string; via: string; field: string }[] };
  expect(g.edges).toContainEqual({ from: s.source.id, to: s.target.id, via: "mention", field: "body" });

  // A click on the card, off its title, opens the note in the pane.
  await card.locator(".query-slot[data-field='blurb']").click();
  await expect(page).toHaveURL(new RegExp(`/notes/${s.target.id}$`));
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(s.targetTitle);
});

test("a card of a deleted note shows it is missing and stays in the content", async ({ page, admin }) => {
  const s = await setup(admin);
  const holder = await admin.put(`/api/notes/${s.source.id}`, {
    data: {
      note: {
        fields: { title: `Card holder ${s.t}` },
        blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "card", fields: { note: s.target.id } }] }],
      },
      base_version: s.source.version,
    },
  });
  expect(holder.status(), await holder.text()).toBe(200);
  const del = await admin.delete(`/api/notes/${s.target.id}?base_version=${s.target.version}`);
  expect(del.ok(), await del.text()).toBe(true);

  await logIn(page, `/notes/${s.source.id}`);
  const card = body(page).getByTestId("note-card");
  await expect(card).toHaveAttribute("data-missing", "true");
  await expect(card).toHaveText("Missing note");
  expect(await storedBody(admin, s.source.id)).toEqual([{ kind: "card", fields: { note: s.target.id } }]);
});
