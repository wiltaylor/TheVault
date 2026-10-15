import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The graph at the top of the right sidebar: the active note in the middle, a node for each note
// it links to or that links to it, with arrows showing which way each link runs. A node opens
// its note in the pane, the graph follows, and a save that adds a link redraws it.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** A type whose notes mention each other in `body`, point one way with `ref` and pair up with `partners`. */
function typeWcl(name: string) {
  return `label  = "Gnode"
plural = "Gnodes"

field body {
  kind  = "content"
  label = "Body"
}

field ref {
  kind   = "reference"
  label  = "Ref"
  target = "${name}"
}

field partners {
  kind     = "reference"
  label    = "Partners"
  target   = "${name}"
  relation = "peer"
  multiple = true
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
`;
}

const para = (text: string) => ({ kind: "paragraph", fields: { text } });

async function post(admin: APIRequestContext, type: string, title: string, fields: Record<string, unknown> = {}, text = "") {
  const res = await admin.post(`/api/notes?type=${type}`, {
    data: {
      note: {
        fields: { title, ...fields },
        blocks: text ? [{ kind: "content", labels: ["body"], blocks: [para(text)] }] : [],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

/**
 * A centre note that mentions `out`, is pointed at by `inn`'s reference, and is a peer of `both`,
 * plus a note `later` nothing links to yet.
 */
async function setup(admin: APIRequestContext) {
  const t = token();
  const type = `gnode_${t}`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl: typeWcl(type) } });
  expect(put.status(), await put.text()).toBe(200);
  const out = await post(admin, type, `Out ${t}`);
  const both = await post(admin, type, `Both ${t}`);
  const later = await post(admin, type, `Later ${t}`);
  const centre = await post(admin, type, `Centre ${t}`, { partners: [both] }, `See [[${out}]] and [[+${type}|Not yet]].`);
  const inn = await post(admin, type, `In ${t}`, { ref: centre });
  return { t, type, centre, out, inn, both, later };
}

const graph = (page: Page) => page.getByTestId("note-graph");
const node = (page: Page, id: string) => graph(page).locator(`[data-node-id="${id}"]`);
const line = (page: Page, a: string, b: string) => graph(page).locator(`line[data-from="${a}"][data-to="${b}"]`);

async function openWithSidebar(page: Page, id: string) {
  await logIn(page, `/notes/${id}`);
  await page.getByRole("button", { name: "Show right sidebar" }).click();
  await expect(graph(page)).toHaveAttribute("data-center", id);
}

test("the graph shows a note's links each way, and a node opens its note and recentres", async ({ page, admin }) => {
  const s = await setup(admin);
  await openWithSidebar(page, s.centre);

  await expect(graph(page).locator("[data-node-id]")).toHaveCount(4);
  await expect(node(page, s.centre)).toHaveAttribute("data-direction", "center");
  await expect(node(page, s.out)).toHaveAttribute("data-direction", "out");
  await expect(node(page, s.inn)).toHaveAttribute("data-direction", "in");
  await expect(node(page, s.both)).toHaveAttribute("data-direction", "both");
  await expect(node(page, s.inn)).toHaveAttribute("aria-label", `Gnode: In ${s.t}, links to this note`);

  // Every line runs from the centre; its arrows say which way the links go.
  const outLine = line(page, s.centre, s.out);
  await expect(outLine).toHaveAttribute("data-direction", "out");
  await expect(outLine).toHaveAttribute("data-forward", "");
  await expect(outLine).not.toHaveAttribute("data-backward", "");
  await expect(outLine).toHaveAttribute("marker-end", /url\(#/);
  const inLine = line(page, s.centre, s.inn);
  await expect(inLine).toHaveAttribute("data-direction", "in");
  await expect(inLine).toHaveAttribute("data-backward", "");
  await expect(inLine).not.toHaveAttribute("data-forward", "");
  await expect(inLine).toHaveAttribute("marker-start", /url\(#/);
  const bothLine = line(page, s.centre, s.both);
  await expect(bothLine).toHaveAttribute("data-direction", "both");
  await expect(bothLine).toHaveAttribute("marker-start", /url\(#/);
  await expect(bothLine).toHaveAttribute("marker-end", /url\(#/);

  // A node is a link: clicking it opens the note in the pane, and the graph centres on it.
  await node(page, s.out).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${s.out}$`));
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(`Out ${s.t}`);
  await expect(graph(page)).toHaveAttribute("data-center", s.out);
  await expect(node(page, s.centre)).toHaveAttribute("data-direction", "in");
  await expect(graph(page).locator("[data-node-id]")).toHaveCount(2);

  // Keyboard: a node takes focus, and Enter opens it.
  await node(page, s.centre).focus();
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(new RegExp(`/notes/${s.centre}$`));
  await expect(graph(page)).toHaveAttribute("data-center", s.centre);
});

test("collapsing the graph lasts across a reload", async ({ page, admin }) => {
  const s = await setup(admin);
  await openWithSidebar(page, s.centre);
  const toggle = page.getByRole("button", { name: "Graph", exact: true });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(graph(page)).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("button", { name: "Graph", exact: true })).toHaveAttribute("aria-expanded", "false");
  await expect(graph(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Graph", exact: true }).click();
  await expect(graph(page)).toHaveAttribute("data-center", s.centre);
});

test("a note with no links says so, and a saved mention adds its node", async ({ page, admin }) => {
  const s = await setup(admin);
  await openWithSidebar(page, s.later);
  await expect(page.getByTestId("graph-empty")).toHaveText("No links yet.");

  const doc = activePane(page).getByRole("textbox", { name: "Body" });
  await doc.click();
  await page.keyboard.type("see [[");
  await expect(page.getByRole("combobox", { name: "Find a note to mention" })).toBeFocused();
  await page.keyboard.type(`Out ${s.t}`);
  await expect(page.getByTestId("mention-result").filter({ hasText: `Out ${s.t}` })).toBeVisible();
  await page.keyboard.press("Enter");
  await activePane(page).getByLabel("Title", { exact: true }).focus();
  await expect(activePane(page).getByTestId("save-status")).toHaveText("Saved");

  await expect(node(page, s.out)).toHaveAttribute("data-direction", "out");
  await expect(line(page, s.later, s.out)).toHaveAttribute("data-direction", "out");
  await expect(page.getByTestId("graph-empty")).toHaveCount(0);
});
