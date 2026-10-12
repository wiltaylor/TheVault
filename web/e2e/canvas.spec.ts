import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

interface Stored {
  id: string;
  version: string;
  note: { fields: Record<string, unknown>; blocks: Block[] };
}

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(
  admin: APIRequestContext,
  type: string,
  fields: Record<string, unknown>,
  blocks: Block[] = [],
): Promise<Stored> {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

function boardCanvas(note: Stored): Block | undefined {
  return note.note.blocks.find((b) => b.kind === "canvas" && b.labels?.[0] === "board");
}

async function openNote(p: Page, id: string, title: string) {
  await logIn(p, `/notes/${id}`);
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
}

async function saveAndWait(p: Page) {
  await p.getByRole("button", { name: "Save" }).click();
  await expect(p.getByRole("button", { name: "Edit" })).toBeVisible();
}

/** What the canvas shows: each node's id, kind and box, and each edge's ends. */
async function drawn(canvas: Locator) {
  const nodes = await canvas.getByTestId("canvas-node").evaluateAll((els) =>
    els.map((el) => {
      const s = (el as HTMLElement).style;
      return [el.getAttribute("data-id"), el.getAttribute("data-kind"), s.left, s.top, s.width, s.height].join(" ");
    }),
  );
  const edges = await canvas
    .getByTestId("canvas-edge")
    .evaluateAll((els) => els.map((el) => [...el.querySelectorAll("line")].map((l) => l.outerHTML).join("")));
  return { nodes, edges };
}

/** The view as the canvas element reports it. */
async function viewOf(canvas: Locator) {
  const v = canvas.getByTestId("canvas-view");
  return {
    x: Number(await v.getAttribute("data-x")),
    y: Number(await v.getAttribute("data-y")),
    zoom: Number(await v.getAttribute("data-zoom")),
  };
}

test("two nodes and an edge made in the editor save a canvas that validates and reloads identically", async ({
  page: p,
  admin,
}) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title });
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas).toBeVisible();
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(0);

  await p.getByRole("button", { name: "Edit" }).click();
  await canvas.getByRole("button", { name: "Add text" }).click();
  await canvas.getByLabel("Node text").fill("First **idea**");
  await canvas.getByLabel("Node text").blur();
  await canvas.getByRole("button", { name: "Add link" }).click();
  await canvas.getByLabel("Link address").fill("https://example.com/plan");
  await canvas.getByRole("button", { name: "Add this link" }).click();
  const nodes = canvas.getByTestId("canvas-node");
  await expect(nodes).toHaveCount(2);

  // Drag the link node away from the text node, then join the two.
  const link = nodes.nth(1);
  const box = (await link.boundingBox())!;
  await p.mouse.move(box.x + 20, box.y + 20);
  await p.mouse.down();
  await p.mouse.move(box.x + 120, box.y + 70, { steps: 5 });
  await p.mouse.up();
  await canvas.getByRole("button", { name: "Connect" }).click();
  await nodes.nth(0).click({ position: { x: 10, y: 10 } });
  await nodes.nth(1).click({ position: { x: 10, y: 10 } });
  await expect(canvas.getByTestId("canvas-edge")).toHaveCount(1);
  await canvas.getByRole("button", { name: "Connect" }).click();

  const sent = p.waitForRequest((r) => r.method() === "PUT" && r.url().endsWith(`/api/notes/${note.id}`));
  await saveAndWait(p);
  const body = (await sent).postDataJSON() as { note: Stored["note"] };
  const saved = body.note.blocks.find((b) => b.kind === "canvas")!;
  expect(saved.blocks?.map((b) => `${b.kind} ${b.labels?.[0] ?? ""}`)).toEqual([
    "viewport ",
    "node text",
    "node link",
    "edge text_to_link",
  ]);

  // The server took it, so it validated, and holds exactly what was sent.
  const after = await stored(admin, note.id);
  expect(after.version).not.toBe(note.version);
  expect(boardCanvas(after)).toEqual(saved);
  expect(saved.blocks?.[1]?.fields?.text).toBe("First **idea**");
  const before = await drawn(canvas);
  await expect(canvas.locator("strong", { hasText: "idea" })).toBeVisible();

  await p.reload();
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
  const reloaded = p.getByTestId("field-board").getByTestId("canvas");
  await expect(reloaded.getByTestId("canvas-node")).toHaveCount(2);
  expect(await drawn(reloaded)).toEqual(before);
  expect(boardCanvas(await stored(admin, note.id))).toEqual(saved);
});

test("pan and zoom are saved with the canvas and come back after a reload", async ({ page: p, admin }) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title }, [
    {
      kind: "canvas",
      labels: ["board"],
      blocks: [
        { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
        { kind: "node", labels: ["a"], fields: { kind: "text", x: 40, y: 40, width: 200, height: 80, text: "Anchor" } },
      ],
    },
  ]);
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("100%");

  await p.getByRole("button", { name: "Edit" }).click();
  await canvas.getByRole("button", { name: "Zoom in" }).click();
  await canvas.getByRole("button", { name: "Zoom in" }).click();
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("156%");
  // Drag an empty part of the canvas to pan.
  const view = canvas.getByTestId("canvas-view");
  const vb = (await view.boundingBox())!;
  await p.mouse.move(vb.x + vb.width - 40, vb.y + vb.height - 40);
  await p.mouse.down();
  await p.mouse.move(vb.x + vb.width - 140, vb.y + vb.height - 90, { steps: 5 });
  await p.mouse.up();
  const panned = await viewOf(canvas);
  expect(panned.x).toBeGreaterThan(50);
  expect(panned.y).toBeGreaterThan(20);
  await saveAndWait(p);

  const vp = boardCanvas(await stored(admin, note.id))?.blocks?.find((b) => b.kind === "viewport")?.fields;
  expect(vp?.zoom).toBeCloseTo(1.5625, 3);
  expect(vp?.x).toBeCloseTo(panned.x, 1);
  expect(vp?.y).toBeCloseTo(panned.y, 1);

  await p.reload();
  const reloaded = p.getByTestId("field-board").getByTestId("canvas");
  await expect(reloaded.getByTestId("canvas-zoom")).toHaveText("156%");
  const back = await viewOf(reloaded);
  expect(back.x).toBeCloseTo(panned.x, 1);
  expect(back.y).toBeCloseTo(panned.y, 1);
  expect(back.zoom).toBeCloseTo(1.5625, 3);
});

test("a canvas on a type's template shows on every note, and its query node follows new notes", async ({
  page: p,
  admin,
}) => {
  const tag = unique("Crew");
  const red = await create(admin, "crew", { title: `${tag} red` });
  const blue = await create(admin, "crew", { title: `${tag} blue` });
  await create(admin, "board", { title: `${tag} red board 1`, crew: red.id });
  await create(admin, "board", { title: `${tag} blue board 1`, crew: blue.id });

  // Every crew shows the template's canvas, and its query node sees that crew as `this`.
  for (const [crew, name] of [
    [blue, "blue"],
    [red, "red"],
  ] as const) {
    if (crew === blue) await openNote(p, crew.id, `${tag} ${name}`);
    else {
      await p.goto(`/notes/${crew.id}`);
      await expect(p.getByRole("heading", { level: 1, name: `${tag} ${name}` })).toBeVisible();
    }
    const canvas = p.getByTestId("canvas");
    await expect(canvas).toHaveCount(1);
    await expect(canvas.getByTestId("canvas-node")).toHaveCount(2);
    await expect(canvas.getByTestId("canvas-edge")).toHaveCount(1);
    await expect(canvas.locator("strong", { hasText: "boards" })).toBeVisible();
    const rows = canvas.getByTestId("canvas-query").getByTestId("query-row");
    await expect(rows).toHaveText([`${tag} ${name} board 1`]);
  }

  // A board made elsewhere for the red crew appears without a reload; the blue crew's does not.
  const rows = p.getByTestId("canvas").getByTestId("canvas-query").getByTestId("query-row");
  await create(admin, "board", { title: `${tag} red board 2`, crew: red.id });
  await create(admin, "board", { title: `${tag} blue board 2`, crew: blue.id });
  await expect(rows).toHaveText([`${tag} red board 1`, `${tag} red board 2`], { timeout: 15_000 });
  await expect(p.getByTestId("canvas-query-count")).toHaveText("2 notes");
});

test("a canvas field placed on a template shows on every note of the type", async ({ page: p, admin }) => {
  const tag = unique("Field");
  const one = await create(admin, "board", { title: `${tag} one` });
  const two = await create(admin, "board", { title: `${tag} two` }, [
    {
      kind: "canvas",
      labels: ["board"],
      blocks: [
        { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
        { kind: "node", labels: ["n"], fields: { kind: "text", x: 0, y: 0, width: 100, height: 60, text: "Two's node" } },
      ],
    },
  ]);
  await openNote(p, one.id, `${tag} one`);
  await expect(p.getByTestId("field-board").getByTestId("canvas")).toBeVisible();
  await expect(p.getByTestId("field-board").getByTestId("canvas-node")).toHaveCount(0);
  await p.goto(`/notes/${two.id}`);
  await expect(p.getByTestId("field-board").getByTestId("canvas-node")).toHaveText(["Two's node"]);
});

test("a canvas inserted into content is edited in place and saved in the content field", async ({ page: p, admin }) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title });
  await openNote(p, note.id, title);
  await p.getByRole("button", { name: "Edit" }).click();
  const body = p.getByTestId("field-body");
  await body.getByRole("button", { name: "Insert canvas" }).click();
  const canvas = body.getByTestId("canvas");
  await expect(canvas).toBeVisible();
  await canvas.getByRole("button", { name: "Add query" }).click();
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(1);
  await canvas.getByLabel("Node query").fill(`from board where title == "${title}"`);
  await canvas.getByLabel("Node query").blur();
  await expect(canvas.getByTestId("query-card")).toHaveCount(1);
  await saveAndWait(p);

  const content = (await stored(admin, note.id)).note.blocks.find((b) => b.kind === "content");
  const inner = content?.blocks?.find((b) => b.kind === "canvas");
  expect(inner?.labels).toBeUndefined();
  expect(inner?.blocks?.find((b) => b.kind === "node")?.fields).toMatchObject({
    kind: "query",
    query: `from board where title == "${title}"`,
  });
  await expect(p.getByTestId("field-body").getByTestId("query-card")).toHaveCount(1);
});

test("the template editor adds a canvas that every note of the type then shows", async ({ page: p, admin }) => {
  // A type of its own, so changing its template touches no other test.
  const name = `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const put = await admin.put(`/api/types/${name}`, {
    data: { wcl: `label = "Temp"\nplural = "Temps"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n` },
  });
  expect(put.status(), await put.text()).toBe(200);
  const a = await create(admin, name, { title: "Temp A" });
  const b = await create(admin, name, { title: "Temp B" });

  await logIn(p, `/types/${name}/templates`);
  await p.getByRole("button", { name: "Add canvas" }).click();
  const canvas = p.getByTestId("canvas");
  await expect(canvas).toBeVisible();
  await canvas.getByRole("button", { name: "Add text" }).click();
  await canvas.getByLabel("Node text").fill("On every temp");
  await canvas.getByLabel("Node text").blur();
  await p.getByRole("button", { name: "Save template" }).click();
  await expect(p.getByText("Saved the default template.")).toBeVisible();

  for (const n of [a, b]) {
    await p.goto(`/notes/${n.id}`);
    await expect(p.getByTestId("canvas").getByTestId("canvas-node")).toHaveText(["On every temp"]);
  }
});
