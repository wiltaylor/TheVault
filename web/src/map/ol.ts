// The OpenLayers side of a map (D29), loaded on first use: it is large, and most pages have no map.
// A real-world map draws its base map from the tile setting: a MapLibre style document through
// ol-mapbox-style, or a raster `{z}/{x}/{y}` template. An image map shows one image in a pixel
// projection the size of the image. Either way pins are DOM buttons placed as overlays, so they
// can be focused, clicked and dragged like any other control.

import { apply } from "ol-mapbox-style";
import OlMap from "ol/Map";
import Overlay from "ol/Overlay";
import View from "ol/View";
import { defaults as defaultControls } from "ol/control/defaults";
import { boundingExtent, getCenter } from "ol/extent";
import LayerGroup from "ol/layer/Group";
import ImageLayer from "ol/layer/Image";
import TileLayer from "ol/layer/Tile";
import { fromLonLat, toLonLat } from "ol/proj";
import Projection from "ol/proj/Projection";
import ImageStatic from "ol/source/ImageStatic";
import XYZ from "ol/source/XYZ";
import "ol/ol.css";
import type { Json } from "../api";
import {
  type GeoPoint,
  type ImagePoint,
  geoValue,
  imageToMap,
  isTileTemplate,
  mapToImage,
} from "./model";

export type Base = { kind: "geo"; tiles: string } | { kind: "image"; url: string; width: number; height: number };

export interface PinView {
  id: string;
  title: string;
  point: GeoPoint | ImagePoint;
}

export interface MapOptions {
  target: HTMLElement;
  base: Base;
  /** Pins can be dragged to a new point. */
  draggable?: boolean;
  /** A click on the map away from any pin, with the stored point under it. */
  onPick?: (value: Json) => void;
  /** A pin starting to move, before it is dropped. */
  onDragStart?: (id: string) => void;
  /** A pin dropped at a new point. */
  onMove?: (id: string, value: Json) => void;
  /** A pin clicked without being dragged. */
  onOpen?: (id: string) => void;
}

export interface MapHandle {
  /** Replace the pins. `fit` frames them all, or the whole map when there are none. */
  setPins(pins: PinView[], fit: boolean): void;
  /** Show `popup` beside pin `id`, or hide it with `null`. */
  showPopup(id: string | null): void;
  readonly popup: HTMLElement;
  setTiles(url: string): void;
  dispose(): void;
}

/** Pointer travel, in CSS pixels, before a press on a pin becomes a drag. */
const DRAG_SLOP = 4;
const FIT_MAX_ZOOM = 14;
const FIT_PADDING = [48, 48, 48, 48];

function baseLayer(tiles: string): LayerGroup {
  if (isTileTemplate(tiles)) {
    return new LayerGroup({
      layers: [new TileLayer({ source: new XYZ({ url: tiles, crossOrigin: "anonymous", maxZoom: 19 }) })],
    });
  }
  const group = new LayerGroup();
  apply(group, tiles).catch((e) => console.warn("the map style did not load", e));
  return group;
}

/** Make a map in `target`. The caller owns `target`; `dispose` leaves it empty. */
export function createMap(opts: MapOptions): MapHandle {
  const { base } = opts;
  let toCoord: (p: GeoPoint | ImagePoint) => number[];
  let toValue: (c: number[]) => Json;
  let view: View;
  let group: LayerGroup;
  let whole: number[];

  if (base.kind === "geo") {
    toCoord = (p) => fromLonLat([(p as GeoPoint).lng, (p as GeoPoint).lat]);
    toValue = (c) => {
      const [lng = 0, lat = 0] = toLonLat(c);
      return { ...geoValue(lng, lat) };
    };
    view = new View({ center: [0, 0], zoom: 1, maxZoom: 19 });
    group = baseLayer(base.tiles);
    whole = [-20037508.34, -15000000, 20037508.34, 15000000];
  } else {
    whole = [0, 0, base.width, base.height];
    const projection = new Projection({ code: "thevault-image", units: "pixels", extent: whole });
    toCoord = (p) => imageToMap(p as ImagePoint, base.height);
    toValue = (c) => ({ ...mapToImage([c[0] ?? 0, c[1] ?? 0], base.width, base.height) });
    view = new View({ projection, center: getCenter(whole), zoom: 1, maxZoom: 8 });
    group = new LayerGroup({
      layers: [new ImageLayer({ source: new ImageStatic({ url: base.url, projection, imageExtent: whole }) })],
    });
  }

  const map = new OlMap({
    target: opts.target,
    layers: [group],
    view,
    // TheVault draws the attribution itself, so it shows whatever serves the tiles.
    controls: defaultControls({ attribution: false, rotate: false }),
  });

  // Tests and tools read where the view is to check where pins are drawn.
  const publishView = () => {
    const c = view.getCenter();
    if (c) opts.target.dataset.center = `${c[0]},${c[1]}`;
    opts.target.dataset.resolution = String(view.getResolution() ?? "");
  };
  map.on("moveend", publishView);

  const pins = new Map<string, { overlay: Overlay; pin: PinView }>();
  let dragging: string | null = null;

  const popup = document.createElement("div");
  popup.className = "map-popup";
  const popupOverlay = new Overlay({
    element: popup,
    positioning: "bottom-center",
    offset: [0, -14],
    stopEvent: true,
    autoPan: { animation: { duration: 0 } },
  });
  map.addOverlay(popupOverlay);

  function pinElement(id: string): HTMLButtonElement {
    const el = document.createElement("button");
    el.type = "button";
    el.className = "map-pin";
    el.dataset.testid = "map-pin";
    el.dataset.note = id;
    if (opts.draggable) el.classList.add("draggable");
    el.addEventListener("pointerdown", (down) => {
      if (down.button !== 0) return;
      down.preventDefault();
      down.stopPropagation();
      el.setPointerCapture(down.pointerId);
      const entry = () => pins.get(id);
      let moved = false;
      const move = (ev: PointerEvent) => {
        if (!opts.draggable) return;
        if (!moved && Math.hypot(ev.clientX - down.clientX, ev.clientY - down.clientY) < DRAG_SLOP) return;
        if (!moved) opts.onDragStart?.(id);
        moved = true;
        dragging = id;
        entry()?.overlay.setPosition(map.getCoordinateFromPixel(map.getEventPixel(ev)));
      };
      const up = (ev: PointerEvent) => {
        el.removeEventListener("pointermove", move);
        el.removeEventListener("pointerup", up);
        el.removeEventListener("pointercancel", up);
        dragging = null;
        if (!moved) {
          opts.onOpen?.(id);
          return;
        }
        const coord = map.getCoordinateFromPixel(map.getEventPixel(ev));
        const value = toValue(coord);
        const e = entry();
        if (e) {
          // Snap to the stored point, which is rounded and kept on the map.
          e.pin = { ...e.pin, point: value as unknown as GeoPoint | ImagePoint };
          e.overlay.setPosition(toCoord(e.pin.point));
        }
        opts.onMove?.(id, value);
      };
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", up);
      el.addEventListener("pointercancel", up);
    });
    // Keyboard users open a pin with Enter or Space, which a button turns into a click.
    el.addEventListener("click", (ev) => {
      if (ev.detail === 0) opts.onOpen?.(id);
    });
    return el;
  }

  map.on("singleclick", (ev) => {
    if (opts.onPick) opts.onPick(toValue(ev.coordinate));
  });

  function setPins(next: PinView[], fit: boolean) {
    const keep = new Set(next.map((p) => p.id));
    for (const [id, e] of pins) {
      if (!keep.has(id)) {
        map.removeOverlay(e.overlay);
        pins.delete(id);
      }
    }
    for (const pin of next) {
      let e = pins.get(pin.id);
      if (!e) {
        const element = pinElement(pin.id);
        const overlay = new Overlay({ element, positioning: "center-center", stopEvent: true });
        map.addOverlay(overlay);
        e = { overlay, pin };
        pins.set(pin.id, e);
      }
      e.pin = pin;
      const el = e.overlay.getElement() as HTMLButtonElement;
      el.title = pin.title || "Untitled";
      el.setAttribute("aria-label", pin.title || "Untitled");
      if (dragging !== pin.id) e.overlay.setPosition(toCoord(pin.point));
    }
    if (fit) {
      const size = map.getSize();
      const coords = next.map((p) => toCoord(p.point));
      if (base.kind === "image" || coords.length === 0) {
        view.fit(whole, { size, padding: base.kind === "image" ? [8, 8, 8, 8] : undefined });
      } else {
        view.fit(boundingExtent(coords), { size, padding: FIT_PADDING, maxZoom: FIT_MAX_ZOOM });
      }
      publishView();
    }
  }

  publishView();
  return {
    setPins,
    popup,
    showPopup(id) {
      const e = id ? pins.get(id) : undefined;
      popupOverlay.setPosition(e ? toCoord(e.pin.point) : undefined);
    },
    setTiles(url) {
      if (base.kind !== "geo") return;
      map.removeLayer(group);
      group = baseLayer(url);
      map.getLayers().insertAt(0, group);
    },
    dispose() {
      map.setTarget(undefined);
      map.dispose();
    },
  };
}
