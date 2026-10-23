// Locations and maps (D3, D29): the stored forms of a location field's value, the map
// coordinates a pin is drawn at, and the tile setting real-world maps fetch their base map from.
// Nothing here loads OpenLayers, so it is cheap to import and to test.

import { createSignal } from "solid-js";
import type { FieldDef, Json, Row } from "../api";

/** OpenFreeMap's default style. A MapLibre style document, not a raster tile template. */
export const DEFAULT_TILES = "https://tiles.openfreemap.org/styles/liberty";

/** The credit a real-world map always shows, whatever serves its tiles. */
export const OSM_ATTRIBUTION = "© OpenStreetMap contributors";
export const OSM_COPYRIGHT = "https://www.openstreetmap.org/copyright";

const TILES_KEY = "thevault.map.tiles";

function storedTiles(): string {
  try {
    return globalThis.localStorage?.getItem(TILES_KEY) || DEFAULT_TILES;
  } catch {
    return DEFAULT_TILES;
  }
}

const [tiles, setTilesSignal] = createSignal(storedTiles());

/** Where real-world maps get their base map: a style URL, or a `{z}/{x}/{y}` raster template. */
export const tileUrl = tiles;

/** Change the tile setting for this browser. Empty text goes back to the default. */
export function setTileUrl(url: string) {
  const next = url.trim() || DEFAULT_TILES;
  try {
    if (next === DEFAULT_TILES) globalThis.localStorage?.removeItem(TILES_KEY);
    else globalThis.localStorage?.setItem(TILES_KEY, next);
  } catch {
    // Storage can be off, as in a private window. The setting then lasts for this page.
  }
  setTilesSignal(next);
}

/** A `map` view's `height`, in screen pixels, as `schema` checks it. */
export const MAP_VIEW_HEIGHT = { initial: 448, min: 160, max: 4000 };

/** The height of a location field's map in a note's editor, in screen pixels. */
export const LOCATION_MAP_HEIGHT = { initial: 256, min: 160, max: 2000 };

const LOCATION_HEIGHT_KEY = "thevault.map.locationHeight";

function locationKey(type: string, field: string): string {
  return `${LOCATION_HEIGHT_KEY}:${type}:${field}`;
}

/**
 * The height this browser keeps for field `field` of type `type`'s map, or the default. It is a
 * display preference, never written to the vault.
 */
export function locationMapHeight(type: string, field: string): number {
  try {
    const n = Number(globalThis.localStorage?.getItem(locationKey(type, field)));
    if (Number.isFinite(n) && n >= LOCATION_MAP_HEIGHT.min && n <= LOCATION_MAP_HEIGHT.max) return Math.round(n);
  } catch {
    // Storage can be off; the default then shows.
  }
  return LOCATION_MAP_HEIGHT.initial;
}

/** Keep `h` as the height of the field's map in this browser; the default forgets it. */
export function setLocationMapHeight(type: string, field: string, h: number) {
  try {
    if (h === LOCATION_MAP_HEIGHT.initial) globalThis.localStorage?.removeItem(locationKey(type, field));
    else globalThis.localStorage?.setItem(locationKey(type, field), String(h));
  } catch {
    // Storage can be off, as in a private window. The height then lasts for this page.
  }
}

/** Whether `url` is a raster tile template rather than a style document. */
export function isTileTemplate(url: string): boolean {
  return url.includes("{z}") && url.includes("{x}") && (url.includes("{y}") || url.includes("{-y}"));
}

/** A tile setting the map can use: an absolute http(s) URL. */
export function validTileUrl(url: string): boolean {
  try {
    const u = new URL(url.replace(/\{-?[a-z]\}/g, "0"));
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

export interface GeoPoint {
  lat: number;
  lng: number;
}

export interface ImagePoint {
  x: number;
  y: number;
}

function record(v: Json | undefined): Record<string, Json> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? v : null;
}

const num = (v: Json | undefined): v is number => typeof v === "number" && Number.isFinite(v);

export function geoPoint(v: Json | undefined): GeoPoint | null {
  const r = record(v);
  return r && num(r.lat) && num(r.lng) ? { lat: r.lat, lng: r.lng } : null;
}

export function imagePoint(v: Json | undefined): ImagePoint | null {
  const r = record(v);
  return r && num(r.x) && num(r.y) ? { x: r.x, y: r.y } : null;
}

const round = (n: number, places: number) => Math.round(n * 10 ** places) / 10 ** places;

/** Degrees to six places, about 10 cm, with the longitude wrapped into -180..180. */
export function geoValue(lng: number, lat: number): GeoPoint {
  const wrapped = ((((lng + 180) % 360) + 360) % 360) - 180;
  return {
    lat: round(Math.max(-90, Math.min(90, lat)), 6),
    lng: round(wrapped === -180 && lng > 0 ? 180 : wrapped, 6),
  };
}

/**
 * An image map's map coordinates have y growing upwards from the bottom edge; a stored point
 * has y growing downwards from the top-left corner. `height` is the image's height in pixels.
 */
export function imageToMap(p: ImagePoint, height: number): [number, number] {
  return [p.x, height - p.y];
}

/** A map coordinate on an image map as a stored point: inside the image, to a tenth of a pixel. */
export function mapToImage(c: [number, number], width: number, height: number): ImagePoint {
  return {
    x: round(Math.max(0, Math.min(width, c[0])), 1),
    y: round(Math.max(0, Math.min(height, height - c[1])), 1),
  };
}

/** What a map of a location field is drawn on. */
export type MapKind = { kind: "geo" } | { kind: "image"; asset: string };

export function mapKind(field: FieldDef | undefined): MapKind {
  return field?.image ? { kind: "image", asset: field.image } : { kind: "geo" };
}

/** A note's pin: its stored point, as `[lng, lat]` on a real-world map or `{x, y}` on an image. */
export interface Pin {
  id: string;
  title: string;
  type: string;
  geo?: GeoPoint;
  image?: ImagePoint;
}

/** The pins of `rows` at field `location`, and how many rows have no point there. */
export function pinsOf(rows: Row[], location: string, kind: MapKind): { pins: Pin[]; missing: number } {
  const pins: Pin[] = [];
  let missing = 0;
  for (const r of rows) {
    const v = r.fields[location];
    const base = { id: r.id, title: r.title, type: r.type };
    if (kind.kind === "geo") {
      const geo = geoPoint(v);
      if (geo) pins.push({ ...base, geo });
      else missing++;
    } else {
      const image = imagePoint(v);
      if (image) pins.push({ ...base, image });
      else missing++;
    }
  }
  return { pins, missing };
}
