import { type JSX, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { ApiError, type Json, type Note, type NoteDoc, type Row, api } from "../api";
import { ConflictView } from "../components/ConflictView";
import type { PagedRows } from "../components/pagedRows";
import { applyEdits } from "../notes";
import { typeDef } from "../session";
import { MapCanvas, type PinView } from "./MapCanvas";
import { MAP_VIEW_HEIGHT, type MapKind, mapKind, pinsOf } from "./model";
import { NewNoteBox, newLabel, useNewNote } from "../views/NewNoteBox";

/** Pins drawn at once. A larger result pages, like the details view (D33). */
export const PINS_PER_PAGE = 500;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

interface Conflict {
  id: string;
  mine: NoteDoc;
  stored: Note;
}

/**
 * A query view's `map` view: a pin per result note at its `location` field (D29). The field
 * decides the map: a real-world map, or the image an image-map field names. Dragging a pin writes
 * the note's field against the version the note had when the drag started, so a save that lands
 * meanwhile answers 409 and opens the conflict view (D12); clicking a pin opens its card. The map
 * is the view's `height` tall; with `onHeight`, a handle on its bottom edge sets that height.
 */
export function MapView(props: {
  location: string | null;
  data: PagedRows;
  total: number;
  card: (row: Row) => JSX.Element;
  /** The view's stored `height`, or null for the default. */
  height?: number | null;
  /** Write the view's new height; the default leaves the item out. Without it, no handle. */
  onHeight?: (h: number) => void;
}) {
  const [page, setPage] = createSignal(0);
  const [open, setOpen] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // Points written by dragging, until the query is run again. `null` is a note with no point.
  const [moved, setMoved] = createSignal<Record<string, Json>>({});
  const [conflict, setConflict] = createSignal<Conflict | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The note as it was when each pin's drag started.
  const reads = new Map<string, Promise<Note>>();

  const pages = () => Math.max(1, Math.ceil(props.total / PINS_PER_PAGE));
  createEffect(on(pages, (n) => page() >= n && setPage(n - 1)));
  createEffect(
    on([page, () => props.total], ([p, total]) => void props.data.loadUntil(Math.min(total, (p + 1) * PINS_PER_PAGE) - 1)),
  );
  const shown = () => props.data.rows().slice(page() * PINS_PER_PAGE, (page() + 1) * PINS_PER_PAGE);

  // With `allow_new`: "+ New" arms the map, the next click picks the spot, and a title box adds
  // a note there.
  const spec = useNewNote();
  const [placing, setPlacing] = createSignal(false);
  const [picked, setPicked] = createSignal<Json | null>(null);

  /** The location field's definition, from the first result whose type has it, else the new notes' type. */
  const field = createMemo(() => {
    const name = props.location;
    if (!name) return undefined;
    for (const r of props.data.rows()) {
      const f = typeDef(r.type)?.fields.find((x) => x.name === name);
      if (f) return f.kind === "location" ? f : undefined;
    }
    const s = spec();
    const f = s ? typeDef(s.type)?.fields.find((x) => x.name === name) : undefined;
    return f?.kind === "location" ? f : undefined;
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

  function startDrag(id: string) {
    const note = api.note(id);
    // Awaited when the pin drops; until then a failed read is not an unhandled rejection.
    note.catch(() => {});
    reads.set(id, note);
  }

  async function save(id: string, mine: NoteDoc, base: string) {
    try {
      await api.updateNote(id, mine, base);
      setConflict(null);
    } catch (e) {
      if (e instanceof ApiError && e.isConflict) {
        setConflict({ id, mine, stored: await api.note(id) });
      } else {
        throw e;
      }
    }
  }

  async function move(id: string, value: Json) {
    const name = props.location;
    if (!name) return;
    const read = reads.get(id) ?? api.note(id);
    reads.delete(id);
    setMoved({ ...moved(), [id]: value });
    setBusy(true);
    try {
      const n = await read;
      await save(id, applyEdits(n.note, { [name]: value }), n.version);
      setError(null);
    } catch (e) {
      const { [id]: _, ...rest } = moved();
      setMoved(rest);
      setError(`The pin did not move: ${message(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function saveMine() {
    const c = conflict();
    if (!c) return;
    setBusy(true);
    setError(null);
    try {
      await save(c.id, c.mine, c.stored.version);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  function keepStored() {
    const c = conflict();
    const name = props.location;
    setConflict(null);
    if (c && name) setMoved({ ...moved(), [c.id]: c.stored.note.fields[name] ?? null });
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
        <Show when={spec() && kind()}>
          <div class="query-new-bar">
            <Show
              when={picked()}
              fallback={
                <button
                  type="button"
                  class="new-note-button"
                  data-testid="new-note-button"
                  aria-pressed={placing()}
                  onClick={() => setPlacing(!placing())}
                >
                  {placing() ? "Click the map where the note goes, or press to cancel" : `+ New ${newLabel(spec()!)}`}
                </button>
              }
            >
              {(at) => (
                <NewNoteBox
                  spec={spec()!}
                  once
                  extra={() => ({ [props.location ?? ""]: at() })}
                  onClose={() => setPicked(null)}
                />
              )}
            </Show>
          </div>
        </Show>
        <Show when={conflict()}>
          {(c) => (
            <ConflictView
              type={typeDef(c().stored.type)}
              mine={c().mine}
              stored={c().stored.note}
              busy={busy()}
              onKeepStored={keepStored}
              onSaveMine={() => void saveMine()}
            />
          )}
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
              height={props.height ?? MAP_VIEW_HEIGHT.initial}
              range={MAP_VIEW_HEIGHT}
              onHeight={props.onHeight}
              draggable
              onDragStart={startDrag}
              onMove={(id, v) => void move(id, v)}
              onOpen={(id) => setOpen(open() === id ? null : id)}
              onPick={(v) => {
                if (!placing()) return;
                setPlacing(false);
                setPicked(v);
              }}
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
