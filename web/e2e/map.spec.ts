import { deflateSync } from "node:zlib";
import type { APIRequestContext, Locator, Page, Request } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Maps (D29): real-world maps over a local tile stand-in, and image maps. Every request a map
// makes to a host other than the server is answered here, so no test reaches the internet.

const OPENFREEMAP = "https://tiles.openfreemap.org";
const CUSTOM_TILES = "http://tiles.standin.test/{z}/{x}/{y}.png";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

/** A solid-colour PNG of `width` by `height`. */
function png(width: number, height: number, [r, g, b]: [number, number, number]): Buffer {
  const table = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (buf: Buffer) => {
    let c = 0xffffffff;
    for (const byte of buf) c = table[(c ^ byte) & 0xff]! ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (kind: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(kind, "ascii"), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc(body), body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: width }, () => [r, g, b]).flat())]);
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(pixels)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const TILE = png(256, 256, [220, 226, 236]);
const IMAGE_WIDTH = 400;
const IMAGE_HEIGHT = 300;

/** A MapLibre style standing in for OpenFreeMap's: one raster source, served below. */
const STANDIN_STYLE = {
  version: 8,
  sources: {
    standin: { type: "raster", tiles: [`${OPENFREEMAP}/standin/{z}/{x}/{y}.png`], tileSize: 256 },
  },
  layers: [
    { id: "background", type: "background", paint: { "background-color": "#e2e8f0" } },
    { id: "standin", type: "raster", source: "standin" },
  ],
};

/**
 * Answer every request leaving the server: OpenFreeMap's style and tiles, and the custom tile
 * host. Anything else is refused. Returns the URLs asked for, in order.
 */
async function standIn(page: Page, server: string): Promise<string[]> {
  const seen: string[] = [];
  await page.route(
    (url) => url.origin !== new URL(server).origin,
    async (route) => {
      const url = route.request().url();
      seen.push(url);
      const cors = { "Access-Control-Allow-Origin": "*" };
      if (url === `${OPENFREEMAP}/styles/liberty`) {
        await route.fulfill({ json: STANDIN_STYLE, headers: cors });
      } else if (url.startsWith(`${OPENFREEMAP}/standin/`) || url.startsWith("http://tiles.standin.test/")) {
        await route.fulfill({ body: TILE, contentType: "image/png", headers: cors });
      } else {
        await route.abort();
      }
    },
  );
  return seen;
}

const tileRequest = /\/\d+\/\d+\/\d+\.png$/;

async function putType(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.ok(), await res.text()).toBe(true);
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: unknown[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

async function fields(admin: APIRequestContext, id: string): Promise<Record<string, unknown>> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

/** A place type whose notes sit on a real-world map, made once per run. */
let placeType: Promise<string> | undefined;
function ensurePlace(admin: APIRequestContext): Promise<string> {
  placeType ??= (async () => {
    const name = `place_${token()}`;
    await putType(
      admin,
      name,
      `label  = "Place"
plural = "Places"

field tag {
  kind  = "string"
  label = "Tag"
}

field spot {
  kind  = "location"
  label = "Spot"
  map   = "geo"
}

template default {
  tab "Main" {
    slot "title"
    slot "spot"
  }
}

template card {
  tab "Card" {
    slot "title"
    slot "tag"
  }
}
`,
    );
    return name;
  })();
  return placeType;
}

/** A room type whose notes sit on an uploaded floor plan, made once per run. */
let roomType: Promise<{ name: string; asset: string }> | undefined;
function ensureRoom(admin: APIRequestContext): Promise<{ name: string; asset: string }> {
  roomType ??= (async () => {
    const upload = await admin.post(`/api/assets?filename=plan-${token()}.png`, {
      data: png(IMAGE_WIDTH, IMAGE_HEIGHT, [0, 200, 0]),
      headers: { "Content-Type": "application/octet-stream" },
    });
    expect(upload.status(), await upload.text()).toBe(201);
    const asset = ((await upload.json()) as { id: string }).id;
    const name = `room_${token()}`;
    await putType(
      admin,
      name,
      `label  = "Room"
plural = "Rooms"

field tag {
  kind  = "string"
  label = "Tag"
}

field desk {
  kind  = "location"
  label = "Desk"
  map   = "image"
  image = "${asset}"
}

template default {
  tab "Main" {
    slot "title"
    slot "desk"
  }
}
`,
    );
    return { name, asset };
  })();
  return roomType;
}

/** A page holding one query view with a single map view. */
async function mapPage(admin: APIRequestContext, title: string, query: string, location: string) {
  const view = {
    kind: "query_view",
    fields: { query },
    blocks: [{ kind: "view", labels: ["map"], fields: { kind: "map", location } }],
  };
  return create(admin, "page", { title }, [{ kind: "content", labels: ["body"], blocks: [view] }]);
}

interface MapFrame {
  box: { x: number; y: number; width: number; height: number };
  center: [number, number];
  resolution: number;
}

/** Where a map's view is, as the map publishes it, and where it sits on the page. */
async function frame(map: Locator): Promise<MapFrame> {
  const target = map.getByTestId("map-target");
  await expect(target).toHaveAttribute("data-resolution", /\d/);
  const box = (await target.boundingBox())!;
  const center = (await target.getAttribute("data-center"))!.split(",").map(Number) as [number, number];
  const resolution = Number(await target.getAttribute("data-resolution"));
  return { box, center, resolution };
}

/** The page position of map coordinate `c`. */
function toScreen(f: MapFrame, c: [number, number]): { x: number; y: number } {
  return {
    x: f.box.x + f.box.width / 2 + (c[0] - f.center[0]) / f.resolution,
    y: f.box.y + f.box.height / 2 - (c[1] - f.center[1]) / f.resolution,
  };
}

/** The map coordinate under page position `p`. */
function toMap(f: MapFrame, p: { x: number; y: number }): [number, number] {
  return [
    f.center[0] + (p.x - f.box.x - f.box.width / 2) * f.resolution,
    f.center[1] - (p.y - f.box.y - f.box.height / 2) * f.resolution,
  ];
}

/** Web Mercator, worked out here rather than taken from the app. */
function mercator(lat: number, lng: number): [number, number] {
  const r = 6378137;
  return [(lng * Math.PI * r) / 180, r * Math.log(Math.tan(Math.PI / 4 + (lat * Math.PI) / 360))];
}

async function centre(el: Locator): Promise<{ x: number; y: number }> {
  const b = (await el.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

function expectNear(actual: { x: number; y: number }, expected: { x: number; y: number }, within = 1.5) {
  expect(Math.abs(actual.x - expected.x), `x ${actual.x} vs ${expected.x}`).toBeLessThanOrEqual(within);
  expect(Math.abs(actual.y - expected.y), `y ${actual.y} vs ${expected.y}`).toBeLessThanOrEqual(within);
}

/** The colour the map draws at page position `p`. */
async function colourAt(map: Locator, p: { x: number; y: number }): Promise<number[]> {
  return map.locator(".ol-layer canvas").first().evaluate((canvas: HTMLCanvasElement, at) => {
    const r = canvas.getBoundingClientRect();
    const sx = canvas.width / r.width;
    const data = canvas.getContext("2d")!.getImageData((at.x - r.left) * sx, (at.y - r.top) * sx, 1, 1).data;
    return [data[0]!, data[1]!, data[2]!];
  }, p);
}

const MELBOURNE = { lat: -37.8136, lng: 144.9631 };
const SYDNEY = { lat: -33.8688, lng: 151.2093 };

test("notes with geo locations render as pins at their coordinates, with attribution", async ({ page, admin, baseURL }) => {
  const place = await ensurePlace(admin);
  const tag = token();
  await create(admin, place, { title: `Melbourne ${tag}`, tag, spot: MELBOURNE });
  await create(admin, place, { title: `Sydney ${tag}`, tag, spot: SYDNEY });
  await create(admin, place, { title: `Nowhere ${tag}`, tag });
  const title = `Places ${tag}`;
  const note = await mapPage(admin, title, `from ${place} where tag == "${tag}" sort title asc`, "spot");

  const seen = await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("query-view").getByTestId("map");
  await expect(map).toHaveAttribute("data-map", "geo");
  const pins = map.getByTestId("map-pin");
  await expect(pins).toHaveCount(2);
  await expect(page.getByTestId("map-missing")).toHaveText("1 note has no spot.");

  const f = await frame(map);
  for (const [name, at] of [
    ["Melbourne", MELBOURNE],
    ["Sydney", SYDNEY],
  ] as const) {
    const pin = map.getByRole("button", { name: `${name} ${tag}` });
    expectNear(await centre(pin), toScreen(f, mercator(at.lat, at.lng)));
  }
  // Sydney is north-east of Melbourne.
  const mel = await centre(map.getByRole("button", { name: `Melbourne ${tag}` }));
  const syd = await centre(map.getByRole("button", { name: `Sydney ${tag}` }));
  expect(syd.x).toBeGreaterThan(mel.x);
  expect(syd.y).toBeLessThan(mel.y);

  // The default base map is OpenFreeMap's style, drawn from the stand-in tiles.
  await expect.poll(() => seen.filter((u) => u.startsWith(`${OPENFREEMAP}/standin/`) && tileRequest.test(u)).length).toBeGreaterThan(0);
  expect(seen).toContain(`${OPENFREEMAP}/styles/liberty`);
  expect(seen.filter((u) => !u.startsWith(OPENFREEMAP))).toEqual([]);

  const attribution = map.getByTestId("map-attribution");
  await expect(attribution).toBeVisible();
  await expect(attribution).toContainText("© OpenStreetMap contributors");
  await expect(attribution.getByRole("link", { name: "OpenStreetMap" })).toHaveAttribute(
    "href",
    "https://www.openstreetmap.org/copyright",
  );

  // A pin opens its note's card.
  await map.getByRole("button", { name: `Melbourne ${tag}` }).click();
  const card = map.getByTestId("map-card");
  await expect(card).toBeVisible();
  await expect(card.getByRole("link", { name: `Melbourne ${tag}` })).toHaveAttribute("href", /\/notes\//);
  await expect(card).toContainText(tag);
});

test("changing the tile URL setting changes the tile requests", async ({ page, admin, baseURL }) => {
  const place = await ensurePlace(admin);
  const tag = token();
  await create(admin, place, { title: `Hobart ${tag}`, tag, spot: { lat: -42.8821, lng: 147.3272 } });
  const title = `Tiles ${tag}`;
  const note = await mapPage(admin, title, `from ${place} where tag == "${tag}"`, "spot");

  const seen = await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("map");
  await expect(map.getByTestId("map-pin")).toHaveCount(1);
  await expect.poll(() => seen.filter((u) => u.startsWith(OPENFREEMAP) && tileRequest.test(u)).length).toBeGreaterThan(0);

  await page.getByRole("link", { name: "Settings" }).click();
  const tiles = page.getByLabel("Tile URL");
  await tiles.fill("not a url");
  await expect(page.getByText("The tile URL must be an absolute http or https URL.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save" })).toBeDisabled();
  await tiles.fill(CUSTOM_TILES);
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("status")).toContainText(CUSTOM_TILES);

  seen.length = 0;
  await page.goto(`/notes/${note.id}`);
  await expect(map.getByTestId("map-pin")).toHaveCount(1);
  await expect.poll(() => seen.filter((u) => u.startsWith("http://tiles.standin.test/") && tileRequest.test(u)).length).toBeGreaterThan(0);
  expect(seen.filter((u) => u.startsWith(OPENFREEMAP))).toEqual([]);
  // The pin stays at its coordinates on the new tiles.
  expectNear(await centre(map.getByTestId("map-pin")), toScreen(await frame(map), mercator(-42.8821, 147.3272)));
  await expect(map.getByTestId("map-attribution")).toContainText("© OpenStreetMap contributors");

  // Going back to the default asks OpenFreeMap again.
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Use OpenFreeMap" }).click();
  seen.length = 0;
  await page.goto(`/notes/${note.id}`);
  await expect(map.getByTestId("map-pin")).toHaveCount(1);
  await expect.poll(() => seen.filter((u) => u.startsWith(OPENFREEMAP) && tileRequest.test(u)).length).toBeGreaterThan(0);
  expect(seen.filter((u) => u.startsWith("http://tiles.standin.test/"))).toEqual([]);
});

test("a pin placed on an image map saves pixel coordinates and reloads at the same spot", async ({ page, admin, baseURL }) => {
  const room = await ensureRoom(admin);
  const title = `Office ${token()}`;
  const note = await create(admin, room.name, { title });

  const seen = await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByRole("region", { name: "Desk map" });
  await expect(map).toHaveAttribute("data-map", "image");
  const f = await frame(map);
  // The image fills the map's middle; its pixels are the stand-in green.
  await expect.poll(() => colourAt(map, { x: f.box.x + f.box.width / 2, y: f.box.y + f.box.height / 2 })).toEqual([0, 200, 0]);
  await expect(map.getByTestId("map-attribution")).toHaveCount(0);

  const click = { x: f.box.x + f.box.width / 2 - 70, y: f.box.y + f.box.height / 2 + 40 };
  await page.mouse.click(click.x, click.y);
  const pin = map.getByTestId("map-pin");
  await expect(pin).toHaveCount(1);
  expectNear(await centre(pin), click);
  const [mx, my] = toMap(f, click);
  const expected = { x: Math.round(mx * 10) / 10, y: Math.round((IMAGE_HEIGHT - my) * 10) / 10 };
  await expect(page.getByRole("group", { name: "Desk" }).getByLabel("X")).toHaveValue(String(expected.x));
  // The pin saves itself.
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  expect((await fields(admin, note.id)).desk).toEqual(expected);

  await page.reload();
  await expect(map.getByTestId("map-pin")).toHaveCount(1);
  expectNear(await centre(map.getByTestId("map-pin")), click);
  expect(seen).toEqual([]);
});

test("an image map view shows the image, and a dragged pin keeps its new pixel coordinates", async ({
  page,
  admin,
  baseURL,
}) => {
  const room = await ensureRoom(admin);
  const tag = token();
  const desk = await create(admin, room.name, { title: `Desk ${tag}`, tag, desk: { x: 100, y: 50 } });
  await create(admin, room.name, { title: `Window ${tag}`, tag, desk: { x: 350.5, y: 260 } });
  const note = await mapPage(admin, `Floor ${tag}`, `from ${room.name} where tag == "${tag}" sort title asc`, "desk");

  const seen = await standIn(page, baseURL!);
  const assetRequests: string[] = [];
  page.on("request", (r: Request) => r.url().includes(`/api/assets/${room.asset}`) && assetRequests.push(r.url()));
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("query-view").getByTestId("map");
  await expect(map).toHaveAttribute("data-map", "image");
  await expect(map.getByTestId("map-pin")).toHaveCount(2);
  expect(assetRequests.length).toBeGreaterThan(0);
  const f = await frame(map);
  await expect.poll(() => colourAt(map, toScreen(f, [200, 150]))).toEqual([0, 200, 0]);

  const deskPin = map.getByRole("button", { name: `Desk ${tag}` });
  const windowPin = map.getByRole("button", { name: `Window ${tag}` });
  expectNear(await centre(deskPin), toScreen(f, [100, IMAGE_HEIGHT - 50]));
  expectNear(await centre(windowPin), toScreen(f, [350.5, IMAGE_HEIGHT - 260]));

  // Drag the desk pin 60 px right and 40 px down.
  const from = await centre(deskPin);
  const to = { x: from.x + 60, y: from.y + 40 };
  const saved = page.waitForResponse((r) => r.request().method() === "PUT" && r.url().endsWith(`/api/notes/${desk.id}`));
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y + 10, { steps: 4 });
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await page.mouse.up();
  expect((await saved).status()).toBe(200);
  const [mx, my] = toMap(f, to);
  const expected = { x: Math.round(mx * 10) / 10, y: Math.round((IMAGE_HEIGHT - my) * 10) / 10 };
  expect((await fields(admin, desk.id)).desk).toEqual(expected);
  await expect(map.getByTestId("map-card")).toHaveCount(0);

  await page.reload();
  await expect(map.getByTestId("map-pin")).toHaveCount(2);
  const g = await frame(map);
  expectNear(await centre(map.getByRole("button", { name: `Desk ${tag}` })), toScreen(g, [expected.x, IMAGE_HEIGHT - expected.y]));
  expectNear(await centre(map.getByRole("button", { name: `Desk ${tag}` })), to);
  expect(seen).toEqual([]);
});

test("a pin dragged over a stale version opens the conflict view and loses neither save", async ({ page, admin, baseURL }) => {
  const room = await ensureRoom(admin);
  const tag = token();
  const desk = await create(admin, room.name, { title: `Desk ${tag}`, tag, desk: { x: 100, y: 50 } });
  const note = await mapPage(admin, `Clash ${tag}`, `from ${room.name} where tag == "${tag}"`, "desk");

  await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("query-view").getByTestId("map");
  const pin = map.getByRole("button", { name: `Desk ${tag}` });
  await expect(pin).toBeVisible();
  const f = await frame(map);

  // Start the drag, then save the note elsewhere before the drop.
  const from = await centre(pin);
  const to = { x: from.x + 60, y: from.y + 40 };
  const loaded = page.waitForResponse((r) => r.url().endsWith(`/api/notes/${desk.id}`) && r.request().method() === "GET");
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y + 10, { steps: 4 });
  await loaded;
  const stored = (await (await admin.get(`/api/notes/${desk.id}`)).json()) as {
    version: string;
    note: { fields: Record<string, unknown>; blocks: unknown[] };
  };
  const res = await admin.put(`/api/notes/${desk.id}`, {
    data: { note: { ...stored.note, fields: { ...stored.note.fields, title: `Desk ${tag} renamed` } }, base_version: stored.version },
  });
  expect(res.status(), await res.text()).toBe(200);
  const rejected = page.waitForResponse((r) => r.request().method() === "PUT" && r.url().endsWith(`/api/notes/${desk.id}`));
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await page.mouse.up();
  expect((await rejected).status()).toBe(409);

  const [mx, my] = toMap(f, to);
  const expected = { x: Math.round(mx * 10) / 10, y: Math.round((IMAGE_HEIGHT - my) * 10) / 10 };
  const conflict = page.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  const row = conflict.locator('tr[data-field="desk"]');
  await expect(row.getByTestId("conflict-mine")).toHaveText(`x: ${expected.x}, y: ${expected.y}`);
  await expect(row.getByTestId("conflict-stored")).toHaveText("x: 100, y: 50");
  await expect(conflict.locator('tr[data-field="title"]').getByTestId("conflict-stored")).toHaveText(`Desk ${tag} renamed`);
  // The other save is not lost: the note keeps it until the user chooses, and the pin keeps the move.
  expect(await fields(admin, desk.id)).toMatchObject({ title: `Desk ${tag} renamed`, desk: { x: 100, y: 50 } });
  // The conflict view sits above the map and moves it down the page, so place the pin by the map.
  expectNear(await centre(pin), toScreen(await frame(map), [expected.x, IMAGE_HEIGHT - expected.y]));
  await expect(page.getByRole("alert")).toHaveCount(0);

  await conflict.getByRole("button", { name: "Save my version" }).click();
  await expect(conflict).toHaveCount(0);
  await expect.poll(async () => (await fields(admin, desk.id)).desk).toEqual(expected);
  expectNear(await centre(pin), toScreen(await frame(map), [expected.x, IMAGE_HEIGHT - expected.y]));
});

test("keeping the stored version after a stale pin drag puts the pin back", async ({ page, admin, baseURL }) => {
  const room = await ensureRoom(admin);
  const tag = token();
  const desk = await create(admin, room.name, { title: `Desk ${tag}`, tag, desk: { x: 100, y: 50 } });
  const note = await mapPage(admin, `Keep ${tag}`, `from ${room.name} where tag == "${tag}"`, "desk");

  await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("query-view").getByTestId("map");
  const pin = map.getByRole("button", { name: `Desk ${tag}` });
  await expect(pin).toBeVisible();
  const f = await frame(map);

  const from = await centre(pin);
  const loaded = page.waitForResponse((r) => r.url().endsWith(`/api/notes/${desk.id}`) && r.request().method() === "GET");
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y + 10, { steps: 4 });
  await loaded;
  // Someone else moves the pin meanwhile.
  const stored = (await (await admin.get(`/api/notes/${desk.id}`)).json()) as {
    version: string;
    note: { fields: Record<string, unknown>; blocks: unknown[] };
  };
  const res = await admin.put(`/api/notes/${desk.id}`, {
    data: { note: { ...stored.note, fields: { ...stored.note.fields, desk: { x: 300, y: 200 } } }, base_version: stored.version },
  });
  expect(res.status(), await res.text()).toBe(200);
  await page.mouse.move(from.x + 60, from.y + 40, { steps: 4 });
  await page.mouse.up();

  const conflict = page.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  await conflict.getByRole("button", { name: "Keep the stored version" }).click();
  await expect(conflict).toHaveCount(0);
  expect((await fields(admin, desk.id)).desk).toEqual({ x: 300, y: 200 });
  const back = toScreen(f, [300, IMAGE_HEIGHT - 200]);
  await expect.poll(async () => {
    const at = await centre(pin);
    return Math.hypot(at.x - back.x, at.y - back.y);
  }).toBeLessThanOrEqual(1.5);
});

/** Drag `handle` down by `dy` screen pixels with the mouse, in steps, and let go. */
async function dragDown(page: Page, handle: Locator, dy: number) {
  await handle.scrollIntoViewIfNeeded();
  const h = (await handle.boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2 + dy, { steps: 8 });
  await page.mouse.up();
}

/** The height OpenLayers draws the map's canvas at, in CSS pixels. */
const drawnHeight = (map: Locator) =>
  map.locator(".ol-viewport").first().evaluate((el) => Math.round(el.getBoundingClientRect().height));

test("a map view's handle makes it taller, saves the view's height, and a reload keeps it", async ({ page, admin, baseURL }) => {
  const place = await ensurePlace(admin);
  const tag = token();
  await create(admin, place, { title: `Perth ${tag}`, tag, spot: { lat: -31.9523, lng: 115.8613 } });
  const note = await mapPage(admin, `Tall ${tag}`, `from ${place} where tag == "${tag}"`, "spot");

  await standIn(page, baseURL!);
  await logIn(page, `/notes/${note.id}`);
  const map = page.getByTestId("query-view").getByTestId("map");
  await expect(map.getByTestId("map-pin")).toHaveCount(1);
  await expect(map).toHaveCSS("height", "448px");
  await expect.poll(() => drawnHeight(map)).toBeGreaterThan(440);
  const handle = page.getByRole("separator", { name: "Map height" });
  await expect(handle).toHaveAttribute("aria-valuenow", "448");

  await dragDown(page, handle, 300);
  await expect(map).toHaveCSS("height", "748px");
  // The map redraws at its new size, not only its box.
  await expect.poll(() => drawnHeight(map)).toBeGreaterThan(740);
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  const res = await admin.get(`/api/notes/${note.id}`);
  const body = (await res.json()) as { note: { blocks: { blocks: { blocks: { fields: Record<string, unknown> }[] }[] }[] } };
  expect(body.note.blocks[0]!.blocks[0]!.blocks[0]!.fields).toEqual({ kind: "map", location: "spot", height: 748 });

  await page.reload();
  const again = page.getByTestId("query-view").getByTestId("map");
  await expect(again).toHaveCSS("height", "748px");
  await expect.poll(() => drawnHeight(again)).toBeGreaterThan(740);
  await expect(again.getByTestId("map-pin")).toHaveCount(1);
});

test("a location field's map is resized from its handle and keeps the height for the type", async ({ page, admin, baseURL }) => {
  const place = await ensurePlace(admin);
  const first = await create(admin, place, { title: `Darwin ${token()}`, spot: { lat: -12.4634, lng: 130.8456 } });
  const second = await create(admin, place, { title: `Cairns ${token()}` });

  await standIn(page, baseURL!);
  await logIn(page, `/notes/${first.id}`);
  const map = page.getByRole("region", { name: "Spot map" });
  await expect(map).toHaveCSS("height", "256px");
  await dragDown(page, page.getByRole("separator", { name: "Spot map height" }), 200);
  await expect(map).toHaveCSS("height", "456px");
  await expect.poll(() => drawnHeight(map)).toBeGreaterThan(450);
  // A display preference: the note is not changed.
  expect((await fields(admin, first.id)).spot).toEqual({ lat: -12.4634, lng: 130.8456 });

  await page.goto(`/notes/${second.id}`);
  await expect(page.getByRole("region", { name: "Spot map" })).toHaveCSS("height", "456px");
  await page.reload();
  await expect(page.getByRole("region", { name: "Spot map" })).toHaveCSS("height", "456px");
  await page.getByRole("separator", { name: "Spot map height" }).dblclick();
  await expect(page.getByRole("region", { name: "Spot map" })).toHaveCSS("height", "256px");
});
