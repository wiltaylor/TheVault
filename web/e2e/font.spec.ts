import { expect, logIn, test } from "./fixtures";

test("code text uses the bundled JetBrains Mono Nerd Font", async ({ page }) => {
  await logIn(page, "/daily");
  const loaded = await page.evaluate(async () => {
    await document.fonts.load('16px "JetBrainsMono Nerd Font Mono"');
    return document.fonts.check('16px "JetBrainsMono Nerd Font Mono"');
  });
  expect(loaded).toBe(true);
  const mono = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--mono"));
  expect(mono).toContain("JetBrainsMono Nerd Font Mono");
});
