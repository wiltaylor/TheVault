import { For, Show, createMemo, createSignal } from "solid-js";
import { ApiError, type Note, type NoteDoc, type Row, api } from "../api";
import { changes } from "../changes";
import { ConflictView } from "../components/ConflictView";
import { createPagedRows } from "../components/pagedRows";
import { applyEdits } from "../notes";
import { taxonomyDef, typeDef, types } from "../session";
import { type ViewDef, viewTemplate } from "./model";
import { type KanbanColumn, groupField, kanbanColumns, narrow } from "./narrow";
import { NoteLink, TemplateBody } from "./parts";
import { NewNoteButton, useNewNote } from "./NewNoteBox";

/** Cards per column per request. A column shows more a page at a time (D33). */
const COLUMN_PAGE = 25;

/** The drag data type of a kanban card: the note's id. */
const CARD_TYPE = "application/x-thevault-note";

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** A card being dragged: its row, its column, and the note as it was when the drag started. */
interface Drag {
  row: Row;
  from: string;
  note: Promise<Note>;
}

interface Conflict {
  row: Row;
  from: string;
  to: string;
  mine: NoteDoc;
  stored: Note;
}

function Column(props: {
  column: KanbanColumn;
  query: string;
  thisId: string | undefined;
  stamp: number;
  view: ViewDef;
  dragging: Drag | null;
  onDragStart: (row: Row, from: string) => void;
  onDragEnd: () => void;
  onDrop: (to: KanbanColumn) => void;
}) {
  const query = () => narrow(props.query, props.column.where);
  const data = createPagedRows(
    (cursor, size) => api.query(query(), cursor, size, props.thisId),
    () => `${query()}\u0000${props.thisId ?? ""}\u0000${props.stamp}`,
    COLUMN_PAGE,
    changes,
  );
  const [over, setOver] = createSignal(false);
  const accepts = () => props.dragging !== null && props.dragging.from !== props.column.key;
  const spec = useNewNote();
  return (
    <section
      class="kanban-column"
      classList={{ over: over() && accepts() }}
      aria-label={props.column.label}
      data-testid="kanban-column"
      data-term={props.column.key}
      onDragOver={(e) => {
        if (!accepts()) return;
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        if (accepts()) props.onDrop(props.column);
      }}
    >
      <header class="kanban-column-head">
        <span class={`term-swatch colour-${props.column.colour ?? "none"}`} aria-hidden="true">
          ●
        </span>
        <h4>{props.column.label}</h4>
        <span class="muted" data-testid="kanban-count">
          {data.total() ?? ""}
        </span>
      </header>
      <Show when={data.error()}>
        <p class="error" role="alert">
          {data.error()}
        </p>
      </Show>
      <div class="kanban-cards">
        <For each={data.rows()}>
          {(row) => (
            <article
              class="query-card kanban-card"
              classList={{ moving: props.dragging?.row.id === row.id }}
              draggable="true"
              data-testid="kanban-card"
              data-id={row.id}
              onDragStart={(e) => {
                e.dataTransfer?.setData(CARD_TYPE, row.id);
                e.dataTransfer?.setData("text/plain", row.title);
                if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
                props.onDragStart(row, props.column.key);
              }}
              onDragEnd={() => props.onDragEnd()}
            >
              <NoteLink row={row} />
              <TemplateBody row={row} template={viewTemplate(typeDef(row.type), props.view.template, "card")} />
            </article>
          )}
        </For>
      </div>
      <Show when={(data.total() ?? 0) > data.rows().length}>
        <button type="button" class="kanban-more" onClick={() => void data.loadUntil(data.rows().length)}>
          Show more ({(data.total() ?? 0) - data.rows().length} left)
        </button>
      </Show>
      <Show when={spec()}>
        {(s) => (
          <NewNoteButton
            spec={s()}
            class="kanban-new"
            label="New"
            extra={() => ({ [props.view.group ?? ""]: props.column.value })}
          />
        )}
      </Show>
    </section>
  );
}

/**
 * Notes in one column per term of a single-term taxonomy field (D7). Each column is the view's
 * query narrowed to its term, paged on its own. Dropping a card on another column writes the
 * field through the API against the version the note had when the drag started, so a save that
 * lands meanwhile answers 409 and opens the conflict view (D12).
 */
export function KanbanView(props: { view: ViewDef; query: string; thisId: string | undefined }) {
  const check = createMemo(() => groupField(types(), props.query, props.view.group));
  // The same columns stay the same objects when the definitions are read again, so a change
  // elsewhere does not remount every column and drop a drag under way.
  const columns = createMemo(
    () => {
      const f = check().field;
      if (!f) return [];
      return kanbanColumns(taxonomyDef(f.taxonomy ?? "")?.terms ?? [], f.name, f.label);
    },
    [],
    { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) },
  );
  const problem = () => {
    const c = check();
    if (c.problem) return c.problem;
    if (c.field && !taxonomyDef(c.field.taxonomy ?? "")) return `The taxonomy ${c.field.taxonomy} does not exist.`;
    return null;
  };

  const [stamps, setStamps] = createSignal<Record<string, number>>({});
  const refresh = (...keys: string[]) =>
    setStamps((s) => {
      const next = { ...s };
      for (const k of keys) next[k] = (next[k] ?? 0) + 1;
      return next;
    });

  const [drag, setDrag] = createSignal<Drag | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [conflict, setConflict] = createSignal<Conflict | null>(null);

  function startDrag(row: Row, from: string) {
    const note = api.note(row.id);
    // A drag that never drops leaves this unread.
    note.catch(() => {});
    setDrag({ row, from, note });
  }

  async function save(row: Row, from: string, to: string, mine: NoteDoc, base: string) {
    try {
      // The write bumps `changes`, which runs every column's query again.
      await api.updateNote(row.id, mine, base);
      setConflict(null);
    } catch (e) {
      if (e instanceof ApiError && e.isConflict) {
        setConflict({ row, from, to, mine, stored: await api.note(row.id) });
      } else {
        throw e;
      }
    }
  }

  async function drop(to: KanbanColumn) {
    const d = drag();
    const f = check().field;
    setDrag(null);
    if (!d || !f) return;
    setBusy(true);
    setError(null);
    try {
      const n = await d.note;
      const mine = applyEdits(n.note, { [f.name]: to.value });
      await save(d.row, d.from, to.key, mine, n.version);
    } catch (e) {
      setError(`Could not move ${d.row.title || "the note"}: ${message(e)}`);
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
      await save(c.row, c.from, c.to, c.mine, c.stored.version);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  function keepStored() {
    const c = conflict();
    setConflict(null);
    if (c) refresh(c.from, c.to);
  }

  return (
    <Show
      when={!problem()}
      fallback={
        <p class="error" role="alert" data-testid="view-problem">
          {problem()}
        </p>
      }
    >
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={conflict()}>
        {(c) => (
          <ConflictView
            type={typeDef(c().row.type)}
            mine={c().mine}
            stored={c().stored.note}
            busy={busy()}
            onKeepStored={keepStored}
            onSaveMine={() => void saveMine()}
          />
        )}
      </Show>
      <div class="kanban" data-testid="kanban" aria-busy={busy()}>
        <For each={columns()}>
          {(column) => (
            <Column
              column={column}
              query={props.query}
              thisId={props.thisId}
              stamp={stamps()[column.key] ?? 0}
              view={props.view}
              dragging={drag()}
              onDragStart={startDrag}
              onDragEnd={() => setTimeout(() => setDrag(null))}
              onDrop={(to) => void drop(to)}
            />
          )}
        </For>
      </div>
    </Show>
  );
}
