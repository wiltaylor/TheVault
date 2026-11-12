// The unified inbox (TV-20): unprocessed captures, incoming mail, incoming messages and missed
// calls in one list, newest first by each item's own time, with filter chips. Sent mail and
// outgoing messages stay out. Done takes each row out, and the sidebar badge counts what All lists.

import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

test("notes without a verdict stay in the held view, not the Inbox badge, and release returns them", async ({ page, admin }) => {
  const heldTitle = unique("TV231 held mail");
  await create(admin, "email", { title: heldTitle, from_raw: "sender@example.com", labels: ["system/INBOX"] });
  const allowedTitle = unique("TV231 allowed mail");
  const allowed = await create(admin, "email", { title: allowedTitle, from_raw: "sam@example.com", labels: ["system/INBOX"] });
  await logIn(page, "/inbox");
  await release(page.request, allowed.id);
  await page.reload();
  await expect(row(page, allowedTitle)).toBeVisible();
  await expect(list(page).getByRole("link", { name: heldTitle, exact: true })).toHaveCount(0);
  await badgeMatches(page, admin);
  await page.getByRole("button", { name: /^Other held \(/ }).click();
  const heldView = page.getByRole("region", { name: "Held items" });
  // A labelled section is a region. Held rows are not in the normal Inbox list.
  const heldRow = heldView.getByRole("listitem").filter({ hasText: heldTitle });
  await expect(heldRow).toHaveCount(1);
  await heldRow.getByRole("button", { name: "Release", exact: true }).click();
  await expect(heldRow).toHaveCount(0);
  await activePane(page).getByRole("navigation", { name: "Inbox views" }).getByRole("button", { name: "Inbox", exact: true }).click();
  await expect(list(page).getByRole("link", { name: heldTitle, exact: true })).toBeVisible();
  await badgeMatches(page, admin);
});

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

test("historical safety controls require counted confirmation for selected held notes", async ({ page, admin }) => {
  const title = unique("TV151 selection");
  await create(admin, "email", { title, from_raw: "sender@example.test", labels: ["system/INBOX"] });
  await logIn(page, "/inbox");
  await page.getByRole("button", { name: /^Other held \(/ }).click();
  const held = page.getByRole("region", { name: "Held items" });
  await held.getByLabel(`Select ${title}`, { exact: true }).check();
  await held.getByRole("button", { name: "Release selected", exact: true }).click();
  await expect(held.getByText("Release 1 held notes? Matching agent triggers may run.")).toBeVisible();
  await expect(held.getByRole("link", { name: title, exact: true })).toBeVisible();
  await held.getByRole("button", { name: "Confirm release", exact: true }).click();
  await expect(held.getByText("Released 1. Failed 0.")).toBeVisible();
  await expect(held.getByRole("link", { name: title, exact: true })).toHaveCount(0);
  const controls = held.getByRole("region", { name: "Historical safety actions" });
  await expect(controls.getByRole("button", { name: "Preview safety pass", exact: true })).toBeDisabled();
  await controls.getByLabel("Owner-entered estimated USD per classifier decision").fill("0");
  await expect(controls.getByRole("button", { name: "Preview safety pass", exact: true })).toBeEnabled();
  await expect(controls.getByText(/Cleared backlog notes do not start agents/)).toBeVisible();
});

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  const note = (await res.json()) as { id: string };
  return note;
}

/** Clear only this fixture, with the same session-only, fingerprint-bound action as the UI. */
async function release(session: APIRequestContext, id: string) {
  const res = await session.get(`/api/notes/${id}`);
  const note = await res.json();
  expect(note.agent_safety.status).toBe("held");
  const released = await session.post(`/api/notes/${id}/agent-safety/release`, { data: { fingerprint: note.agent_safety.fingerprint } });
  expect(released.ok(), await released.text()).toBe(true);
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
  const mail = await create(admin, "email", { title: mailIn, from_raw: "sam@example.com", sent: at(3600), labels: ["system/INBOX"], snippet: "Lunch on Friday?" });
  const mailSent = `${tag} mail sent`;
  await create(admin, "email", { title: mailSent, sent: at(3500), labels: ["system/SENT"] });
  const smsIn = `${tag} sms in`;
  const sms = await create(admin, "message", { title: smsIn, channel: "sms", direction: "in", from_raw: "+61400000003", sent: at(2400) });
  const smsOut = `${tag} sms out`;
  await create(admin, "message", { title: smsOut, channel: "sms", direction: "out", sent: at(2300) });
  const missed = `${tag} missed call`;
  const call = await create(admin, "call", { title: missed, direction: "missed", number: "+61400000004", started: at(1200), device_ref: `call:${tag}` });
  await create(admin, "call", { title: `${tag} answered call`, direction: "in", started: at(1100), device_ref: `call:${tag}:in` });

  await logIn(page, "/inbox");
  for (const note of [mail, sms, call]) await release(page.request, note.id);
  await page.reload();
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
