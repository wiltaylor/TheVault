import { type JSX, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { type Json, assetUrl } from "../api";
import { type MapKind, OSM_COPYRIGHT, tileUrl } from "./model";
import type { MapHandle, PinView } from "./ol";

export type { PinView };

type Ol = typeof import("./ol");
let loading: Promise<Ol> | undefined;

function ol(): Promise<Ol> {
  loading ??= import("./ol");
  return loading;
}

/** The pixel size of an image, once it has loaded. */
async function imageSize(url: string): Promise<{ width: number; height: number }> {
  const img = new Image();
  img.src = url;
  await img.decode();
  return { width: img.naturalWidth, height: img.naturalHeight };
}

/**
 * One map, real-world or image, with `pins` on it. The view frames the pins when the map opens
 * and whenever `frame` changes; other pin changes leave the view where the user put it.
 */
export function MapCanvas(props: {
  map: MapKind;
  pins: PinView[];
  frame?: unknown;
  label: string;
  class?: string;
  draggable?: boolean;
  onPick?: (value: Json) => void;
  onDragStart?: (id: string) => void;
  onMove?: (id: string, value: Json) => void;
  onOpen?: (id: string) => void;
  /** Pin whose popup is open, and what the popup shows. */
  open?: string | null;
  popup?: JSX.Element;
}) {
  let target!: HTMLDivElement;
  const [handle, setHandle] = createSignal<MapHandle>();
  const [error, setError] = createSignal<string | null>(null);
  let disposed = false;

  onMount(async () => {
    const kind = props.map;
    try {
      const [mod, base] = await Promise.all([
        ol(),
        kind.kind === "geo"
          ? Promise.resolve({ kind: "geo" as const, tiles: tileUrl() })
          : imageSize(assetUrl(kind.asset)).then((size) => ({
              kind: "image" as const,
              url: assetUrl(kind.asset),
              ...size,
            })),
      ]);
      if (disposed) return;
      setHandle(
        mod.createMap({
          target,
          base,
          draggable: props.draggable,
          onPick: (v) => props.onPick?.(v),
          onDragStart: (id) => props.onDragStart?.(id),
          onMove: (id, v) => props.onMove?.(id, v),
          onOpen: (id) => props.onOpen?.(id),
        }),
      );
    } catch {
      if (!disposed) setError(kind.kind === "image" ? "The map image did not load." : "The map did not load.");
    }
  });
  onCleanup(() => {
    disposed = true;
    handle()?.dispose();
  });

  let framed: { handle: MapHandle; frame: unknown } | undefined;
  createEffect(() => {
    const h = handle();
    const pins = props.pins;
    const frame = props.frame;
    if (!h) return;
    const fit = framed?.handle !== h || framed.frame !== frame;
    framed = { handle: h, frame };
    h.setPins(pins, fit);
  });
  createEffect(on(tileUrl, (url) => handle()?.setTiles(url), { defer: true }));
  createEffect(() => handle()?.showPopup(props.open ?? null));

  return (
    <div
      class={`map ${props.class ?? ""}`}
      data-testid="map"
      data-map={props.map.kind}
      role="region"
      aria-label={props.label}
    >
      <div ref={target} class="map-target" data-testid="map-target" />
      <Show when={error()}>
        <p class="error map-error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={props.map.kind === "geo"}>
        <div class="map-attribution" data-testid="map-attribution">
          © <a href={OSM_COPYRIGHT} target="_blank" rel="noopener noreferrer">OpenStreetMap</a> contributors
        </div>
      </Show>
      <Show when={handle()}>
        {(h) => (
          <Show when={props.open}>
            <Portal mount={h().popup}>{props.popup}</Portal>
          </Show>
        )}
      </Show>
    </div>
  );
}
