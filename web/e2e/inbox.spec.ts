// The unified inbox (TV-20): unprocessed captures, incoming mail, incoming messages and missed
// calls in one list, newest first by each item's own time, with filter chips. Sent mail and
// outgoing messages stay out. Done takes each row out, and the sidebar badge counts what All lists.

import type { APIRequestContext, Page } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { activePane, expect, logIn, test, vaultDir } from "./fixtures";

let decisions: Server;
let originalClassifier: string;
test.beforeAll(async ({ admin }) => {
  decisions = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => { body += chunk; });
    req.on("end", () => {
      const reason = body.includes("TV231 held scam") ? "scam" : "allow";
      const probabilities = Object.fromEntries(["allow", "prompt_injection", "sensitive_credentials", "scam", "spam", "marketing"].map((r) => [r, r === reason ? 1 : 0]));
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({ id: "inbox-safety", model: "jev-1.13.0", answers: { reason: { type: "choice", choice: reason, confidence: 1, probabilities } } }));
    });
  });
  await new Promise<void>((resolve) => decisions.listen(0, "127.0.0.1", resolve));
  const port = (decisions.address() as { port: number }).port;
  const account = await admin.post("/api/integrations/ai/accounts", { data: { name: "Inbox decisions", kind: "openai_compatible", base_url: `http://127.0.0.1:${port}/api/v1`, key: "test-decisions-key" } });
  expect(account.ok()).toBe(true);
  const path = join(vaultDir(), "classifiers/agent_safety.wcl");
  originalClassifier = readFileSync(path, "utf8");
  writeFileSync(path, `account = "Inbox decisions"\n${originalClassifier}`);
});

test("held scams stay in the held view, not the Inbox badge, and release returns them", async ({ page, admin }) => {
  const held = await create(admin, "email", { title: unique("TV231 held scam"), from_raw: "scam@example.com", labels: ["system/INBOX"] });
  const allowedTitle = unique("TV231 allowed mail");
  await create(admin, "email", { title: allowedTitle, from_raw: "sam@example.com", labels: ["system/INBOX"] });
  await logIn(page, "/inbox");
  await expect(row(page, allowedTitle)).toBeVisible();
  await badgeMatches(page, admin);
  await page.getByRole("button", { name: /^Possible scams \(/ }).click();
  const heldView = page.getByRole("region", { name: "Held items" });
  // A labelled section is a region. Held rows are not in the normal Inbox list.
  const heldRow = heldView.getByRole("listitem").filter({ hasText: "TV231 held scam" });
  await expect(heldRow).toHaveCount(1);
  await heldRow.getByRole("button", { name: "Release", exact: true }).click();
  await expect(heldRow).toHaveCount(0);
  await activePane(page).getByRole("navigation", { name: "Inbox views" }).getByRole("button", { name: "Inbox", exact: true }).click();
  await expect(list(page).getByRole("link", { name: /TV231 held scam/ })).toBeVisible();
  await badgeMatches(page, admin);
  expect(held.id).toBeTruthy();
});
test.afterAll(async () => {
  if (originalClassifier) writeFileSync(join(vaultDir(), "classifiers/agent_safety.wcl"), originalClassifier);
  await new Promise<void>((resolve) => decisions.close(() => resolve()));
});

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  const note = (await res.json()) as { id: string };
  const incoming = (type === "email" && !(fields.labels as string[] | undefined)?.includes("system/SENT"))
    || (type === "message" && fields.direction === "in") || (type === "call" && fields.direction === "missed");
  if (incoming) {
    await expect.poll(async () => {
      const response = await admin.get(`/api/notes/${note.id}`);
      return (await response.json()).agent_safety?.status;
    }).toBe(fields.title?.toString().includes("TV231 held scam") ? "held" : "allowed");
  }
  return note;
}

/** What the badge counts: the All view (src/inbox/sources.ts). */
const INBOX_QUERIES = [
  "from fleeting where !processed",
  'from email where !processed && !under(labels, "system/SENT")',
  'from message where !processed && direction == "in"',
  'from call where !processed && direction == "missed"',
];

async function inboxTotal(admin: APIRequestContext): Promise<number> {
  const held = new Set<string>();
  let offset = 0;
  while (true) {
    const res = await admin.get(`/api/agent-safety/held?limit=500&offset=${offset}`);
    const page = await res.json();
    for (const note of page.notes) held.add(note.id);
    offset += page.notes.length;
    if (!page.notes.length || offset >= page.total) break;
  }
  let n = 0;
  for (const query of INBOX_QUERIES) {
    let cursor = null;
    do {
      const res = await admin.post("/api/query", { data: { query, cursor, page_size: 500 } });
      const page = await res.json();
      n += page.rows.filter((r: { id: string }) => !held.has(r.id)).length;
      cursor = page.next;
    } while (cursor);
  }
  return n;
}

const inboxLink = (page: Page) => page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: /^Inbox/ });
const list = (page: Page) => activePane(page).getByRole("list", { name: "Inbox" });
/** The rows whose title holds `tag`, in their order on the page. */
const rows = (page: Page, tag: string) => list(page).getByRole("listitem").filter({ hasText: tag });
const chip = (page: Page, name: string) => activePane(page).getByRole("group", { name: "Show" }).getByRole("button", { name, exact: true });
const row = (page: Page, title: string) =>
  list(page).getByRole("listitem").filter({ has: page.getByRole("link", { name: title, exact: true }) });

/** Other specs write notes into the same vault in parallel, so the badge is compared, not counted. */
async function badgeMatches(page: Page, _admin: APIRequestContext) {
  await expect
    .poll(async () => {
      const badge = (await inboxLink(page).textContent())?.replace("Inbox", "").trim();
      return badge === String(await inboxTotal(page.request));
    })
    .toBe(true);
}

test("the inbox lists captures, incoming mail and messages and missed calls by their own time, and Done takes each out", async ({
  page,
  admin,
}) => {
  const tag = unique("Inbox");
  const at = (seconds: number) => new Date(Date.now() + seconds * 1000).toISOString();
  // The times are ahead of every other spec's notes, so these rows are on the first page.
  const capture = `${tag} capture`;
  await create(admin, "fleeting", { title: capture });
  const mailIn = `${tag} mail in`;
  await create(admin, "email", { title: mailIn, from_raw: "sam@example.com", sent: at(3600), labels: ["system/INBOX"], snippet: "Lunch on Friday?" });
  const mailSent = `${tag} mail sent`;
  await create(admin, "email", { title: mailSent, sent: at(3500), labels: ["system/SENT"] });
  const smsIn = `${tag} sms in`;
  await create(admin, "message", { title: smsIn, channel: "sms", direction: "in", from_raw: "+61400000003", sent: at(2400) });
  const smsOut = `${tag} sms out`;
  await create(admin, "message", { title: smsOut, channel: "sms", direction: "out", sent: at(2300) });
  const missed = `${tag} missed call`;
  await create(admin, "call", { title: missed, direction: "missed", number: "+61400000004", started: at(1200), device_ref: `call:${tag}` });
  await create(admin, "call", { title: `${tag} answered call`, direction: "in", started: at(1100), device_ref: `call:${tag}:in` });

  await logIn(page, "/inbox");
  await expect(chip(page, "All")).toHaveAttribute("aria-pressed", "true");
  // Four rows: the mail, the message and the call ahead of the capture, made just now.
  await expect(rows(page, tag)).toHaveCount(4);
  await expect(rows(page, tag).locator(".inbox-title")).toHaveText([
    mailIn,
    smsIn,
    missed,
    capture,
  ]);
  await badgeMatches(page, admin);

  // Each row has Done and Open; only the capture converts, and the mail has no trash.
  for (const title of [capture, mailIn, smsIn, missed]) {
    await expect(row(page, title).getByRole("button", { name: "Done" })).toBeVisible();
    await expect(row(page, title).getByRole("link", { name: "Open" })).toBeVisible();
  }
  await expect(row(page, capture).getByRole("button", { name: "Convert to…" })).toBeVisible();
  for (const title of [mailIn, smsIn, missed]) {
    await expect(row(page, title).getByRole("button", { name: "Convert to…" })).toHaveCount(0);
  }
  await expect(row(page, mailIn).getByRole("button", { name: "Delete" })).toHaveCount(0);
  await expect(row(page, smsIn).getByRole("button", { name: "Delete" })).toBeVisible();
  await expect(row(page, mailIn)).toContainText("sam@example.com");
  await expect(row(page, missed)).toContainText("Missed call");

  // The chips narrow the list to one source.
  for (const [name, title] of [
    ["Captures", capture],
    ["Mail", mailIn],
    ["Messages", smsIn],
    ["Calls", missed],
  ] as const) {
    await chip(page, name).click();
    await expect(chip(page, name)).toHaveAttribute("aria-pressed", "true");
    await expect(rows(page, tag)).toHaveCount(1);
    await expect(row(page, title)).toBeVisible();
  }
  await chip(page, "All").click();
  await expect(rows(page, tag)).toHaveCount(4);

  // Show processed still lists only incoming items, so the sent mail and the outgoing SMS stay out.
  await page.getByLabel("Show processed").check();
  await expect(row(page, mailIn)).toBeVisible();
  await expect(row(page, mailSent)).toHaveCount(0);
  await expect(row(page, smsOut)).toHaveCount(0);
  await page.getByLabel("Show processed").uncheck();

  // Done takes each row out, and the badge follows.
  for (const title of [mailIn, smsIn, missed, capture]) {
    await row(page, title).getByRole("button", { name: "Done" }).click();
    await expect(row(page, title)).toHaveCount(0);
  }
  await expect(rows(page, tag)).toHaveCount(0);
  await badgeMatches(page, admin);
  for (const [type, title] of [
    ["email", mailIn],
    ["message", smsIn],
    ["call", missed],
    ["fleeting", capture],
  ]) {
    const res = await admin.post("/api/query", { data: { query: `from ${type} where title == "${title}"`, page_size: 1 } });
    const found = (await res.json()) as { rows: { fields: Record<string, unknown> }[] };
    expect(found.rows[0]?.fields.processed, title).toBe(true);
  }
});
