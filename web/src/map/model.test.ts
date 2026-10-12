import { beforeEach, describe, expect, it } from "vitest";
import type { Row } from "../api";
import {
  DEFAULT_TILES,
  geoPoint,
  geoValue,
  imagePoint,
  imageToMap,
  isTileTemplate,
  mapKind,
  mapToImage,
  pinsOf,
  setTileUrl,
  tileUrl,
  validTileUrl,
} from "./model";

const row = (id: string, fields: Row["fields"]): Row => ({ id, type: "place", title: id, fields });

describe("tile setting", () => {
  beforeEach(() => setTileUrl(""));

  it("defaults to OpenFreeMap and persists a change", () => {
    expect(tileUrl()).toBe(DEFAULT_TILES);
    setTileUrl("http://tiles.example/{z}/{x}/{y}.png");
    expect(tileUrl()).toBe("http://tiles.example/{z}/{x}/{y}.png");
    expect(localStorage.getItem("thevault.map.tiles")).toBe("http://tiles.example/{z}/{x}/{y}.png");
  });

  it("goes back to the default on empty text", () => {
    setTileUrl("http://tiles.example/{z}/{x}/{y}.png");
    setTileUrl("   ");
    expect(tileUrl()).toBe(DEFAULT_TILES);
    expect(localStorage.getItem("thevault.map.tiles")).toBeNull();
  });

  it("tells a raster template from a style URL", () => {
    expect(isTileTemplate("https://tile.openstreetmap.org/{z}/{x}/{y}.png")).toBe(true);
    expect(isTileTemplate(DEFAULT_TILES)).toBe(false);
  });

  it("accepts only absolute http(s) URLs", () => {
    expect(validTileUrl("https://tile.openstreetmap.org/{z}/{x}/{y}.png")).toBe(true);
    expect(validTileUrl(DEFAULT_TILES)).toBe(true);
    expect(validTileUrl("tiles/{z}/{x}/{y}.png")).toBe(false);
    expect(validTileUrl("javascript:alert(1)")).toBe(false);
  });
});

describe("points", () => {
  it("reads stored points and rejects anything else", () => {
    expect(geoPoint({ lat: -37.8136, lng: 144.9631 })).toEqual({ lat: -37.8136, lng: 144.9631 });
    expect(geoPoint({ lat: "1", lng: 2 })).toBeNull();
    expect(geoPoint(null)).toBeNull();
    expect(imagePoint({ x: 412, y: 188.5 })).toEqual({ x: 412, y: 188.5 });
    expect(imagePoint({ lat: 1, lng: 2 })).toBeNull();
  });

  it("rounds and wraps a picked geo point", () => {
    expect(geoValue(144.96312345, -37.81361234)).toEqual({ lat: -37.813612, lng: 144.963123 });
    expect(geoValue(190, 95)).toEqual({ lat: 90, lng: -170 });
    expect(geoValue(-200, 0)).toEqual({ lat: 0, lng: 160 });
    expect(geoValue(180, 0).lng).toBe(180);
  });

  it("flips y between an image point and map coordinates and back", () => {
    expect(imageToMap({ x: 412, y: 188.5 }, 600)).toEqual([412, 411.5]);
    expect(mapToImage([412, 411.5], 800, 600)).toEqual({ x: 412, y: 188.5 });
  });

  it("keeps a picked image point inside the image", () => {
    expect(mapToImage([-5, 700], 800, 600)).toEqual({ x: 0, y: 0 });
    expect(mapToImage([900, -1], 800, 600)).toEqual({ x: 800, y: 600 });
    expect(mapToImage([10.04, 589.96], 800, 600)).toEqual({ x: 10, y: 10 });
  });
});

describe("pins", () => {
  it("takes a geo field's map from the field", () => {
    expect(mapKind({ image: null } as never)).toEqual({ kind: "geo" });
    expect(mapKind({ image: "01JB9A0C3D4E5F6G7H8J9K0M1Q" } as never)).toEqual({
      kind: "image",
      asset: "01JB9A0C3D4E5F6G7H8J9K0M1Q",
    });
  });

  it("pins rows with a point and counts the rest", () => {
    const rows = [
      row("a", { place: { lat: 1, lng: 2 } }),
      row("b", {}),
      row("c", { place: { x: 1, y: 2 } }),
    ];
    const geo = pinsOf(rows, "place", { kind: "geo" });
    expect(geo.pins.map((p) => [p.id, p.geo])).toEqual([["a", { lat: 1, lng: 2 }]]);
    expect(geo.missing).toBe(2);
    const image = pinsOf(rows, "place", { kind: "image", asset: "x" });
    expect(image.pins.map((p) => [p.id, p.image])).toEqual([["c", { x: 1, y: 2 }]]);
    expect(image.missing).toBe(2);
  });
});
