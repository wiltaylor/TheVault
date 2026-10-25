import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// `<type>/<text>` in a note picker looks for the text among one type's notes only: in the `@`
// picker of the content editor and in a card block's note search. While the type is being typed,
// the picker offers it as "filter by type", which types `<type>/` and leaves the picker open.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const typeWcl = (label: string) => `label = "${label}"
plural = "${label}s"

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
`;

interface Made {
  id: string;
  version: string;
}

/** Two types, each with a note titled with the same words, and a note to edit. */
async function setup(admin: APIRequestContext) {
  const t = token();
  const make = async (name: string, label: string) => {
    const put = await admin.put(`/api/types/${name}`, { data: { wcl: typeWcl(label) } });
    expect(put.status(), await put.text()).toBe(200);
  };
  const post = async (type: string, title: string): Promise<Made> => {
    const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks: [] } } });
    expect(res.status(), await res.text()).toBe(201);
    return (await res.json()) as Made;
  };
  const alpha = `tfalpha_${t}`;
  const beta = `tfbeta_${t}`;
  await make(alpha, `Alpha ${t}`);
  await make(beta, `Beta ${t}`);
  const words = `Filtered ${t}`;
  const inAlpha = await post(alpha, `${words} one`);
  const inBeta = await post(beta, `${words} two`);
  const source = await post(alpha, `Holder ${t}`);
  return { t, alpha, beta, words, inAlpha, inBeta, source };
}

async function bodyBlocks(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  const n = (await res.json()) as { note: { blocks: { kind: string; labels?: string[]; blocks?: { kind: string; fields?: Record<string, unknown> }[] }[] } };
  return n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks ?? [];
}

const body = (page: Page) => activePane(page).getByRole("textbox", { name: "Body" });

test("<type>/ in the @ picker and a card's search finds only that type's notes", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.source.id}`);
  await body(page).click();
  const kb = page.keyboard;

  // Unfiltered, the words find a note of each type.
  await kb.type(`@${s.words}`);
  await expect(page.getByTestId("mention-result").filter({ hasText: s.words })).toHaveCount(2);
  await kb.press("Escape");
  await kb.press("ControlOrMeta+a");
  await kb.press("Delete");

  // The type offered while it is typed: choosing it types `<type>/` and keeps the picker open.
  await kb.type(`@tfbeta_${s.t}`);
  const offer = page.getByTestId("mention-filter").and(page.locator(`[data-type="${s.beta}"]`));
  await expect(offer).toContainText(`Beta ${s.t}`);
  await offer.click();
  const box = page.getByRole("combobox", { name: "Find a note to mention" });
  await expect(box).toHaveValue(`${s.beta}/`);
  await expect(box).toBeFocused();
  await expect(page.getByTestId("mention-type-filter")).toHaveText(`Beta ${s.t}`);
  await kb.type(s.words);
  const results = page.getByTestId("mention-result");
  await expect(results.filter({ hasText: `${s.words} two` })).toBeVisible();
  await expect(results).toHaveCount(1);
  await kb.press("Enter");
  await expect(body(page).getByTestId("mention")).toHaveText(`${s.words} two`);
  await kb.type(" ");

  // In a card's search, the label works as well as the name.
  await kb.type("/card");
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "Note card" }).click();
  const search = body(page).getByRole("combobox", { name: "Note to show as a card" });
  await expect(search).toBeFocused();
  await kb.type(`Alpha ${s.t} / ${s.words}`);
  await expect(body(page).getByTestId("combo-badge")).toHaveText(`Alpha ${s.t}`);
  const options = body(page).getByRole("option");
  await expect(options).toHaveText([`${s.words} oneAlpha ${s.t}`]);
  await options.first().click();
  await expect(body(page).getByTestId("note-card-title")).toHaveText(`${s.words} one`);

  await activePane(page).getByLabel("Title", { exact: true }).focus();
  await expect(activePane(page).getByTestId("save-status")).toHaveText("Saved");
  await expect
    .poll(() => bodyBlocks(admin, s.source.id))
    .toEqual([
      { kind: "paragraph", fields: { text: `[[${s.inBeta.id}]] ` } },
      { kind: "card", fields: { note: s.inAlpha.id } },
    ]);
});
