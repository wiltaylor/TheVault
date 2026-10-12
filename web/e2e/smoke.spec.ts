import { expect, test } from "./fixtures";

test("the built app loads", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  await page.goto("/");

  await expect(page).toHaveTitle("TheVault");
  await expect(page.getByRole("heading", { name: "TheVault" })).toBeVisible();
  expect(errors).toEqual([]);
});
