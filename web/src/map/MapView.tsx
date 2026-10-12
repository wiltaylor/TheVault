import { type JSX, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { type Json, type Row, api } from "../api";
import type { PagedRows } from "../components/pagedRows";
import { typeDef } from "../session";
import { MapCanvas, type PinView } from "./MapCanvas";
import { type MapKind, mapKind, pinsOf } from "./model";

/** Pins drawn at once. A larger result pages, like the details view (D33). */
export const PINS_PER_PAGE = 500;

/**
 * A query view's `map` view: a pin per result note at its `location` field (D29). The field
 * decides the map: a real-world map, or the image an image-map field names. Dragging a pin writes
 * the note's field; clicking one opens its card.
 */
export function MapView(props: {
  location: string | null;
  data: PagedRows;
  total: number;
  card: (row: Row) => JSX.Element;
}) {
  const [page, setPage] = createSignal(0);
  const [open, setOpen] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // Points written by dragging, until the query is run again.
  const [moved, setMoved] = createSignal<Record<string, Json>>({});

  const pages = () => Math.max(1, Math.ceil(props.total / PINS_PER_PAGE));
  createEffect(on(pages, (n) => page() >= n && setPage(n - 1)));
  createEffect(
    on([page, () => props.total], ([p, total]) => void props.data.loadUntil(Math.min(total, (p + 1) * PINS_PER_PAGE) - 1)),
  );
  const shown = () => props.data.rows().slice(page() * PINS_PER_PAGE, (page() + 1) * PINS_PER_PAGE);

  /** The location field's definition, from the first result whose type has it. */
  const field = createMemo(() => {
    const name = props.location;
    if (!name) return undefined;
    for (const r of props.data.rows()) {
      const f = typeDef(r.type)?.fields.find((x) => x.name === name);
      if (f) return f.kind === "location" ? f : undefined;
    }
    return undefined;
  });
  const kind = createMemo<MapKind | undefined>(
    () => (field() ? mapKind(field()) : undefined),
    undefined,
    { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) },
  );

  const placed = createMemo(() => {
    const k = kind();
    const name = props.location;
    if (!k || !name) return { pins: [] as PinView[], missing: 0 };
    const over = moved();
    const rows = shown().map((r) => (r.id in over ? { ...r, fields: { ...r.fields, [name]: over[r.id]! } } : r));
    const { pins, missing } = pinsOf(rows, name, k);
    return {
      pins: pins.map((p) => ({ id: p.id, title: p.title, point: (p.geo ?? p.image)! })),
      missing,
    };
  });

  const openRow = () => shown().find((r) => r.id === open());

  async function move(id: string, value: Json) {
    const name = props.location;
    if (!name) return;
    setMoved({ ...moved(), [id]: value });
    try {
      const note = await api.note(id);
      await api.updateNote(id, { ...note.note, fields: { ...note.note.fields, [name]: value } }, note.version);
      setError(null);
    } catch (e) {
      const { [id]: _, ...rest } = moved();
      setMoved(rest);
      setError(`The pin did not move: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  return (
    <div class="query-map">
      <Show
        when={props.location}
        fallback={<p class="muted">This map view has no location field. Choose one in its settings.</p>}
      >
        <Show when={pages() > 1}>
          <nav class="pager" aria-label="Pin pages">
            <button type="button" onClick={() => setPage(page() - 1)} disabled={page() === 0}>
              Previous pins
            </button>
            <span class="muted">
              Pins {page() * PINS_PER_PAGE + 1} to {Math.min(props.total, (page() + 1) * PINS_PER_PAGE)} of{" "}
              {props.total}
            </span>
            <button type="button" onClick={() => setPage(page() + 1)} disabled={page() >= pages() - 1}>
              Next pins
            </button>
          </nav>
        </Show>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <Show
          when={kind()}
          keyed
          fallback={
            <Show when={props.data.rows().length > 0}>
              <p class="muted">
                No result has a location field named “{props.location}”.
              </p>
            </Show>
          }
        >
          {(k) => (
            <MapCanvas
              map={k}
              pins={placed().pins}
              frame={page()}
              label="Map"
              class="query-map-canvas"
              draggable
              onMove={(id, v) => void move(id, v)}
              onOpen={(id) => setOpen(open() === id ? null : id)}
              open={openRow() ? open() : null}
              popup={
                <Show when={openRow()}>
                  {(row) => (
                    <article class="query-card map-card" data-testid="map-card">
                      <button type="button" class="map-card-close" aria-label="Close" onClick={() => setOpen(null)}>
                        ×
                      </button>
                      {props.card(row())}
                    </article>
                  )}
                </Show>
              }
            />
          )}
        </Show>
        <Show when={placed().missing > 0}>
          <p class="muted" data-testid="map-missing">
            {placed().missing === 1 ? "1 note has" : `${placed().missing} notes have`} no {props.location}.
          </p>
        </Show>
      </Show>
    </div>
  );
}
