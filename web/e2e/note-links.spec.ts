import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Links to other notes open them when clicked, wherever they show: in a note's always-editable
// content, a template's fixed blocks, a reference field's chips, a canvas card and a version
// preview. A plain click opens the note in the pane, Ctrl/Cmd+click and middle-click open a
// background tab, and a link to another site opens in a new browser tab.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** Markup with a mention of `id`, a link to it and a link to another site. */
const linksTo = (id: string) => `See [[${id}]], [the target](/notes/${id}) and [elsewhere](https://example.com/elsewhere).`;

const para = (text: string) => ({ kind: "paragraph", fields: { text } });

/**
 * A type whose template shows the links in a fixed paragraph and in a fixed canvas's card,
 * places the body, and leaves the `ref` reference field to the Properties panel.
 */
function typeWcl(target: string) {
  const text = JSON.stringify(linksTo(target));
  return `label = "Linker"
plural = "Linkers"

field body {
  kind  = "content"
  label = "Body"
}

field ref {
  kind   = "reference"
  label  = "Ref"
  target = "page"
}

template default {
  tab "Main" {
    slot "title"

    paragraph {
      text = ${text}
    }

    canvas {
      height = 200

      viewport {
        x    = 0.0
        y    = 0.0
        zoom = 1.0
      }

      node card {
        kind   = "card"
        x      = 20.0
        y      = 20.0
        width  = 420.0
        height = 120.0

        paragraph {
          text = ${text}
        }
      }
    }

    slot "body"
  }
}
`;
}

interface Made {
  id: string;
  version: string;
}

async function post(admin: APIRequestContext, type: string, note: unknown): Promise<Made> {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Made;
}

/** A page to link to, and a linker note whose body and template link to it. */
async function setup(admin: APIRequestContext) {
  const t = token();
  const targetText = `Target body ${t}`;
  const target = await post(admin, "page", {
    fields: { title: `Target ${t}` },
    blocks: [{ kind: "content", labels: ["body"], blocks: [para(targetText)] }],
  });
  const type = `linker_${t}`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl: typeWcl(target.id) } });
  expect(put.status(), await put.text()).toBe(200);
  const source = await post(admin, type, {
    fields: { title: `Source ${t}`, ref: target.id },
    blocks: [{ kind: "content", labels: ["body"], blocks: [para(linksTo(target.id))] }],
  });
  return { target, targetText, source };
}

const tabs = (page: Page) => page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");

type Setup = Awaited<ReturnType<typeof setup>>;

/** The note's always-editable body. */
const editor = (page: Page) => activePane(page).getByRole("textbox", { name: "Body" });
/** The template's fixed paragraph, its first block. */
const fixed = (page: Page) => activePane(page).getByTestId("template-block").first();
/** The card on the template's fixed canvas. */
const card = (page: Page) => activePane(page).getByTestId("canvas-card");

/** The active pane now shows the target note, in the same tab. */
async function opened(page: Page, s: Setup) {
  await expect(page).toHaveURL(new RegExp(`/notes/${s.target.id}$`));
  await expect(activePane(page).getByTestId("field-body")).toContainText(s.targetText);
  await expect(tabs(page)).toHaveCount(1);
  expect(page.context().pages()).toHaveLength(1);
}

/** Ctrl/Cmd+click and middle-click on `link` each open the target in a background tab and leave the pane alone. */
async function opensBackgroundTabs(page: Page, s: Setup, link: () => Locator) {
  await expect(tabs(page)).toHaveCount(1);
  await link().click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs(page)).toHaveCount(2);
  await link().click({ button: "middle" });
  await expect(tabs(page)).toHaveCount(3);
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
  await expect(tabs(page).nth(1)).toHaveAccessibleName(/^Target /);
  await expect(tabs(page).nth(2)).toHaveAccessibleName(/^Target /);
  expect(page.context().pages()).toHaveLength(1);
}

/** A click on `link` opens its site in a new browser tab, and the pane stays where it is. */
async function opensExternal(page: Page, s: Setup, link: Locator) {
  // The fixture keeps every test off the internet; this one site answers.
  await page.context().route("https://example.com/**", (route) => route.fulfill({ contentType: "text/html", body: "<p>Elsewhere</p>" }));
  const popup = page.context().waitForEvent("page");
  await link.click();
  const p = await popup;
  await p.waitForLoadState();
  expect(p.url()).toBe("https://example.com/elsewhere");
  await p.close();
  await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
  await expect(tabs(page)).toHaveCount(1);
}

test.describe("in the always-editable body", () => {
  test("a click on a mention opens the note in the pane", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await editor(page).getByTestId("mention").click();
    await opened(page, s);
  });

  test("a click on a link to a note opens it in the pane", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await editor(page).getByRole("link", { name: "the target" }).click();
    await opened(page, s);
  });

  test("Ctrl+click and middle-click on a mention open background tabs", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensBackgroundTabs(page, s, () => editor(page).getByTestId("mention"));
  });

  test("Ctrl+click and middle-click on a link to a note open background tabs, and paste nothing", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    // The text once the mention shows its title, which loads after the page.
    await expect(editor(page).getByTestId("mention")).toHaveText(/^Target /);
    const before = await editor(page).textContent();
    await opensBackgroundTabs(page, s, () => editor(page).getByRole("link", { name: "the target" }));
    await expect(editor(page)).toHaveText(before!);
  });

  test("a click on a link to another site opens it in a new browser tab", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensExternal(page, s, editor(page).getByRole("link", { name: "elsewhere" }));
  });

  test("a link stays editable: a drag across it selects it rather than opening it, and the bubble edits it", async ({
    page,
    admin,
  }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    const doc = editor(page);
    const link = doc.getByRole("link", { name: "the target" });
    const bubble = page.getByRole("toolbar", { name: "Body formatting" });
    // The mention before the link shows its title, which loads after the page, before the link is measured.
    await expect(doc.getByTestId("mention")).toHaveText(/^Target /);
    const b = (await link.boundingBox())!;
    // A drag across the link's text selects it and opens nothing.
    await page.mouse.move(b.x + 2, b.y + b.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width - 2, b.y + b.height / 2, { steps: 5 });
    await page.mouse.up();
    await expect(bubble).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
    await expect(bubble.getByRole("button", { name: "Link" })).toHaveAttribute("aria-pressed", "true");
    await bubble.getByRole("button", { name: "Link" }).click();
    await expect(page.getByLabel("Link address")).toHaveValue(`/notes/${s.target.id}`);
    await page.getByLabel("Link address").fill("https://example.com/changed");
    await page.getByLabel("Link address").press("Enter");
    await expect(link).toHaveAttribute("href", "https://example.com/changed");
    await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
  });
});

test.describe("in a template's fixed block", () => {
  test("a click on a mention or a link to a note opens it in the pane", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await fixed(page).getByTestId("mention").click();
    await opened(page, s);
    await page.goBack();
    await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
    await fixed(page).getByRole("link", { name: "the target" }).click();
    await opened(page, s);
  });

  test("Ctrl+click and middle-click on a link to a note open background tabs", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensBackgroundTabs(page, s, () => fixed(page).getByRole("link", { name: "the target" }));
  });

  test("a click on a link to another site opens it in a new browser tab", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensExternal(page, s, fixed(page).getByRole("link", { name: "elsewhere" }));
  });
});

test.describe("in a canvas card", () => {
  test("a click on a mention or a link to a note opens it in the pane", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await card(page).getByTestId("mention").click();
    await opened(page, s);
    await page.goBack();
    await expect(page).toHaveURL(new RegExp(`/notes/${s.source.id}$`));
    await card(page).getByRole("link", { name: "the target" }).click();
    await opened(page, s);
  });

  test("Ctrl+click and middle-click on a link to a note open background tabs", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensBackgroundTabs(page, s, () => card(page).getByRole("link", { name: "the target" }));
  });

  test("a click on a link to another site opens it in a new browser tab", async ({ page, admin }) => {
    const s = await setup(admin);
    await logIn(page, `/notes/${s.source.id}`);
    await opensExternal(page, s, card(page).getByRole("link", { name: "elsewhere" }));
  });
});

test("a reference field's chip in the Properties panel opens the note; Ctrl+click and middle-click open background tabs", async ({
  page,
  admin,
}) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.source.id}`);
  await activePane(page).getByRole("button", { name: "Properties", exact: true }).click();
  const chip = () => page.getByRole("complementary", { name: "Properties" }).getByRole("link", { name: /^Target / });
  await opensBackgroundTabs(page, s, chip);
  await chip().click();
  await expect(page).toHaveURL(new RegExp(`/notes/${s.target.id}$`));
  await expect(activePane(page).getByTestId("field-body")).toContainText(s.targetText);
});

test("in a version preview, a click on a link to a note opens it in the pane", async ({ page, admin }) => {
  const s = await setup(admin);
  // A second version, so the first, with the links, can be previewed.
  const cur = (await (await admin.get(`/api/notes/${s.source.id}`)).json()) as { version: string; note: { fields: Record<string, unknown> } };
  const put = await admin.put(`/api/notes/${s.source.id}`, {
    data: { note: { fields: cur.note.fields, blocks: [{ kind: "content", labels: ["body"], blocks: [para("Changed")] }] }, base_version: cur.version },
  });
  expect(put.status(), await put.text()).toBe(200);
  await logIn(page, `/notes/${s.source.id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  const versions = page.getByRole("complementary", { name: "History" }).getByRole("list", { name: "Versions" }).getByRole("listitem");
  await versions.nth(1).getByRole("button").first().click();
  await expect(page.getByRole("region", { name: "Version preview" })).toBeVisible();
  await activePane(page).getByTestId("field-body").getByRole("link", { name: "the target" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${s.target.id}$`));
  await expect(activePane(page).getByTestId("field-body")).toContainText(s.targetText);
  expect(page.context().pages()).toHaveLength(1);
});
