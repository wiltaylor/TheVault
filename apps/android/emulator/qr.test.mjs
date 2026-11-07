// Tests for qr.mjs (TV-67): run with `node --test apps/android/emulator/qr.test.mjs`. No device needed.

import assert from "node:assert/strict";
import { test } from "node:test";
import { PNG } from "pngjs";
import { qrPng } from "./qr.mjs";

// Pixel (x, y) of a decoded PNG as [r, g, b].
function pixel(png, x, y) {
  const i = (png.width * y + x) * 4;
  return [png.data[i], png.data[i + 1], png.data[i + 2]];
}

test("qrPng returns a PNG of the code with the qrencode -s 8 geometry", async () => {
  const buf = await qrPng("TheVault check 10");
  assert.deepEqual([...buf.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const png = PNG.sync.read(buf);
  assert.equal(png.width, png.height);
  assert.equal(png.width % 8, 0, "every module is 8 px");
  // At least version 1 (21 modules) plus the 4-module quiet zone on each side.
  assert.ok(png.width >= 8 * (21 + 2 * 4), `width ${png.width} is too small for a QR code`);
});

test("qrPng keeps the quiet zone light and draws the finder pattern dark", async () => {
  const png = PNG.sync.read(await qrPng("TheVault check 10"));
  assert.deepEqual(pixel(png, 0, 0), [255, 255, 255], "the quiet zone is white");
  // The top-left finder pattern starts at module (4, 4) with its dark outer ring.
  const edge = 4 * 8;
  assert.deepEqual(pixel(png, edge + 1, edge + 1), [0, 0, 0], "the finder pattern is dark");
});

test("qrPng refuses an empty or non-string text", async () => {
  await assert.rejects(qrPng(""), /non-empty string/);
  await assert.rejects(qrPng(undefined), /non-empty string/);
});
