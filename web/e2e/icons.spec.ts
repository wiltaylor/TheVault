import { expect, test } from "./fixtures";

// The favicons, the Apple touch icon and the manifest's icons all resolve, with the type and the
// size they claim.

function pngSize(body: Buffer): [number, number] {
  expect(body.subarray(1, 4).toString("latin1")).toBe("PNG");
  return [body.readUInt32BE(16), body.readUInt32BE(20)];
}

test("the page's icon links resolve", async ({ page, request }) => {
  await page.goto("/login");
  const links = await page
    .locator('link[rel="icon"], link[rel="apple-touch-icon"]')
    .evaluateAll((els) =>
      els.map((el) => ({
        rel: el.getAttribute("rel"),
        href: el.getAttribute("href")!,
        sizes: el.getAttribute("sizes"),
      })),
    );
  expect(links.map((l) => l.href)).toEqual(["/favicon.ico", "/icon-32.png", "/icon-192.png", "/apple-touch-icon.png"]);
  for (const link of links) {
    const res = await request.get(link.href);
    expect(res.status(), link.href).toBe(200);
    const body = await res.body();
    if (link.href.endsWith(".ico")) {
      expect(res.headers()["content-type"]).toBe("image/x-icon");
      expect(body.readUInt16LE(2)).toBe(1); // an icon resource
      expect(body.readUInt16LE(4)).toBe(3); // 16, 32 and 48
    } else {
      expect(res.headers()["content-type"]).toBe("image/png");
      const [w, h] = pngSize(body);
      expect(`${w}x${h}`).toBe(link.sizes);
    }
  }
});

test("the manifest's icons resolve, with a maskable one", async ({ request }) => {
  const manifest = await (await request.get("/manifest.webmanifest")).json();
  expect(manifest.theme_color).toBe("#0b1020");
  const icons: { src: string; sizes: string; type: string; purpose: string }[] = manifest.icons;
  expect(icons.map((i) => i.purpose).sort()).toEqual(["any", "any", "maskable"]);
  for (const icon of icons) {
    const res = await request.get(icon.src);
    expect(res.status(), icon.src).toBe(200);
    expect(res.headers()["content-type"]).toBe(icon.type);
    const [w, h] = pngSize(await res.body());
    expect(`${w}x${h}`).toBe(icon.sizes);
  }
});

test("the login page shows the app icon", async ({ page }) => {
  await page.goto("/login");
  const logo = page.locator("img.login-logo");
  await expect(logo).toBeVisible();
  expect(await logo.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(192);
});
