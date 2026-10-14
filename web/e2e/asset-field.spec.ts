import { expect, logIn, test } from "./fixtures";

// An asset field is filled by picking an uploaded file, or by uploading a new one from the field.

const FILE = { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("asset field upload\n") };

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

test("an asset field uploads a file into itself: single replaces, multiple appends", async ({ page, admin }) => {
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

  const send = async (label: string, name: string) => {
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: `Upload a file to ${label}` }).click();
    await (await chooser).setFiles({ ...FILE, name });
  };
  const fields = async () =>
    ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;

  await send("Cover", "cover.txt");
  await expect.poll(async () => (await fields()).cover).toMatch(/^[0-9A-Z]{26}$/);
  const cover = (await fields()).cover as string;
  await expect(page.getByLabel("Cover", { exact: true })).toHaveValue(cover);
  await expect(page.getByLabel("Cover", { exact: true }).locator("option:checked")).toHaveText("cover.txt");

  await send("Attachments", "first.txt");
  await expect(page.getByTestId("field-attachments").locator(".chip")).toHaveText(["first.txt×"]);
  await send("Attachments", "second.txt");
  await expect.poll(async () => ((await fields()).attachments as string[] | undefined)?.length).toBe(2);
  await expect(page.getByTestId("field-attachments").locator(".chip")).toHaveText(["first.txt×", "second.txt×"]);
  expect((await fields()).cover).toBe(cover);
});
