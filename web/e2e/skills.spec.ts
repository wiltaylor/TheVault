import type { Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The Skills page and the skill screen (skills S4, S5): open the Skills page from the Skills item
// under Agents in the sidebar, import a zipped skill from its Import menu, see it opened and
// listed, change its description on the Instructions tab with one Save, and find the change in
// SKILL.md, with its script kept.

function agentsNav(page: Page) {
  return page.getByRole("navigation", { name: "Agents" });
}

const CRC = (() => {
  const t: number[] = [];
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t.push(c >>> 0);
  }
  return t;
})();

function crc32(data: Buffer): number {
  let c = 0xffffffff;
  for (const b of data) c = CRC[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of `files`, stored without compression. */
function zip(files: Record<string, string>): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [path, text] of Object.entries(files)) {
    const name = Buffer.from(path, "utf8");
    const data = Buffer.from(text, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0);
    dir.writeUInt16LE(20, 4);
    dir.writeUInt16LE(20, 6);
    dir.writeUInt16LE(0x0800, 8);
    dir.writeUInt32LE(crc, 16);
    dir.writeUInt32LE(data.length, 20);
    dir.writeUInt32LE(data.length, 24);
    dir.writeUInt16LE(name.length, 28);
    dir.writeUInt32LE(offset, 42);
    parts.push(local, name, data);
    central.push(dir, name);
    offset += 30 + name.length + data.length;
  }
  const size = central.reduce((n, b) => n + b.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(size, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, ...central, end]);
}

test("a zipped skill imported from the Skills page is listed, and its description is edited", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = `e2e-skill-${stamp}`;
  const buffer = zip({
    [`${name}/SKILL.md`]: `---\nname: ${name}\ndescription: Fills in forms. Use when a form arrives.\nlicense: MIT\n---\n\n# Forms\n\nRun scripts/fill.sh.\n`,
    [`${name}/scripts/fill.sh`]: "echo filled\n",
  });

  await logIn(page, "/daily");
  const skillsItem = agentsNav(page).getByRole("link", { name: /^Skills/ });
  await skillsItem.click();
  await expect(page).toHaveURL(/\/skills$/);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Skills" })).toBeVisible();
  // The sidebar lists no skill by name.
  await expect(agentsNav(page).getByRole("link", { name })).toHaveCount(0);

  await pane.getByRole("button", { name: "Import" }).click();
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("menu", { name: "Import skill" }).getByRole("menuitem", { name: "From a zip" }).click();
  await (await chooser).setFiles({ name: `${name}.zip`, mimeType: "application/zip", buffer });

  await expect(page).toHaveURL(new RegExp(`/skills/${name}$`));
  await expect(pane.getByRole("heading", { name })).toBeVisible();

  // The Skills page lists it, with its description and license.
  await skillsItem.click();
  await expect(page).toHaveURL(/\/skills$/);
  const card = pane.getByRole("list", { name: "Skills" }).getByRole("listitem").filter({ has: page.getByRole("link", { name, exact: true }) });
  await expect(card).toContainText("Fills in forms. Use when a form arrives.");
  await expect(card).toContainText("2 files");
  await expect(card).toContainText("MIT");
  await card.getByRole("link", { name, exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/skills/${name}$`));

  const instructions = pane.getByRole("region", { name: "Instructions" });
  await expect(instructions.getByLabel("Name")).toHaveValue(name);
  await expect(instructions.getByLabel("License")).toHaveValue("MIT");
  const description = instructions.getByLabel("Description");
  await expect(description).toHaveValue("Fills in forms. Use when a form arrives.");
  await description.fill("Fills in PDF forms. Use when a PDF form arrives.");
  await instructions.getByRole("button", { name: "Save" }).click();
  await expect(instructions.getByRole("status")).toHaveText("Saved.");

  const res = await admin.get(`/api/skills/${name}/files/SKILL.md`);
  expect(res.status(), await res.text()).toBe(200);
  const text = await res.text();
  expect(text).toContain("description: Fills in PDF forms. Use when a PDF form arrives.");
  expect(text).toContain("license: MIT");
  expect(text).toContain("Run scripts/fill.sh.");

  // The script is in the Files tab's tree, and opens as text.
  await pane.getByRole("tab", { name: "Files" }).click();
  const files = pane.getByRole("region", { name: "Files" });
  await files.getByRole("button", { name: "fill.sh", exact: true }).click();
  await expect(files.getByLabel("Contents of scripts/fill.sh")).toHaveValue("echo filled\n");

  // After a reload the change is still there.
  await page.reload();
  await activePane(page).getByRole("tab", { name: "Instructions" }).click();
  await expect(activePane(page).getByRole("region", { name: "Instructions" }).getByLabel("Description")).toHaveValue(
    "Fills in PDF forms. Use when a PDF form arrives.",
  );

  const del = await admin.delete(`/api/skills/${name}`);
  expect(del.ok(), await del.text()).toBe(true);
});
