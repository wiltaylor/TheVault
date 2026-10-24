// The browser box's desktop as an in-app page (browser B4): it opens from the command palette in
// the active pane. The e2e server runs with no box, so the page says how to turn it on.
import { expect, logIn, test } from "./fixtures";

test("Open browser box shows the browser box page in the pane, which says how to set the box up", async ({ page }) => {
  await logIn(page, "/daily");
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await palette.getByRole("combobox").fill("browser box");
  await palette.getByRole("option", { name: /Open browser box/ }).click();
  await expect(page).toHaveURL(/\/browser-box$/);
  const box = page.getByRole("region", { name: "Browser box" });
  await expect(box.getByRole("heading", { name: "Browser box" })).toBeVisible();
  await expect(box.getByText(/Not set up/)).toBeVisible();
  await expect(box.getByTitle("Browser box desktop")).toHaveCount(0);
});
