import { expect, logIn, test } from "./fixtures";

// An asset field lists its files, each removable, and Add + uploads new ones or chooses ones in the vault.

const FILE = { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("asset field upload\n") };

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

test("an asset field uploads files and chooses vault files: single replaces, multiple appends, remove keeps the file", async ({ page, admin }) => {
  const type = `filed_${token()}`;
  const wcl =
    `label = "Filed"\n\nfield cover {\n  kind = "asset"\n  label = "Cover"\n}\n\nfield attachments {\n  kind = "asset"\n  multiple = true\n  label = "Attachments"\n}\n\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "cover"\n    slot "attachments"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title: "Filed note" }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  const { id } = (await res.json()) as { id: string };
  await logIn(page, `/notes/${id}`);

  const send = async (label: string, ...names: string[]) => {
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: `Add an attachment to ${label}` }).click();
    await page.getByRole("menuitem", { name: "Upload a file…" }).click();
    await (await chooser).setFiles(names.map((name) => ({ ...FILE, name })));
  };
  const fields = async () =>
    ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
  const rows = (field: string) => page.getByTestId(`field-${field}`).locator(".asset-row .asset-row-name");

  await send("Cover", "cover.txt");
  await expect.poll(async () => (await fields()).cover).toMatch(/^[0-9A-Z]{26}$/);
  const cover = (await fields()).cover as string;
  await expect(rows("cover")).toHaveText(["cover.txt"]);
  await expect(page.getByTestId("field-cover").getByRole("link", { name: "cover.txt" })).toHaveAttribute("href", `/assets/${cover}`);

  // A multiple field takes several files in one pick, then appends more.
  await send("Attachments", "first.txt", "second.txt");
  await expect.poll(async () => ((await fields()).attachments as string[] | undefined)?.length).toBe(2);
  await expect(rows("attachments")).toHaveText(["first.txt", "second.txt"]);
  await send("Attachments", "third.txt");
  await expect.poll(async () => ((await fields()).attachments as string[] | undefined)?.length).toBe(3);
  await expect(rows("attachments")).toHaveText(["first.txt", "second.txt", "third.txt"]);
  expect((await fields()).cover).toBe(cover);

  // Remove takes a file out of the field; the file stays in the vault.
  const second = ((await fields()).attachments as string[])[1]!;
  await page.getByTestId("field-attachments").getByRole("button", { name: "Remove second.txt" }).click();
  await expect.poll(async () => (await fields()).attachments).toHaveLength(2);
  await expect(rows("attachments")).toHaveText(["first.txt", "third.txt"]);
  expect((await admin.get(`/api/assets/${second}`)).status()).toBe(200);

  // Choose from the vault puts it back, and in a single field replaces the file.
  await page.getByRole("button", { name: "Add an attachment to Attachments" }).click();
  await page.getByRole("menuitem", { name: "Choose from the vault…" }).click();
  const dialog = page.getByRole("dialog", { name: "Choose from the vault for Attachments" });
  await dialog.getByRole("searchbox", { name: "Search files" }).fill("second.txt");
  await dialog.getByRole("button", { name: "second.txt" }).first().click();
  await dialog.getByRole("button", { name: "Add", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(async () => (await fields()).attachments).toHaveLength(3);
  await expect(rows("attachments")).toHaveText(["first.txt", "third.txt", "second.txt"]);

  await page.getByRole("button", { name: "Add an attachment to Cover" }).click();
  await expect(page.getByRole("menu")).toContainText("Replace cover.txt");
  await page.getByRole("menuitem", { name: "Choose from the vault…" }).click();
  const single = page.getByRole("dialog", { name: "Choose from the vault for Cover" });
  await single.getByRole("searchbox", { name: "Search files" }).fill("first.txt");
  await single.getByRole("button", { name: "first.txt" }).first().click();
  await expect(single).toHaveCount(0);
  await expect.poll(async () => (await fields()).cover).not.toBe(cover);
  await expect(rows("cover")).toHaveText(["first.txt"]);
});
