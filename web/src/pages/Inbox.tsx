// The inbox: fleeting notes waiting to be processed, newest first. Each can be opened, marked
// done, converted into a note of another type, or moved to the trash.

import { useNavigate } from "@solidjs/router";
import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import { type Note, type Row, api, assetUrl } from "../api";
import { FLEETING, INBOX_QUERY, PROCESSED, assetIds, blocksText, captureFields, convertDoc } from "../capture/model";
import { openCapture } from "../capture/store";
import { changes } from "../changes";
import { ChipPicker, typeSuggestions } from "../components/ChipPicker";
import { showToast } from "../components/Toasts";
import { knowTitle } from "../editor/titles";
import { applyEdits } from "../notes";
import { typeDef, types } from "../session";
import { errorText } from "./TypeEditor";

const PAGE = 50;

/** Mark fleeting note `id` processed or not, over its current version. */
async function setProcessed(id: string, value: boolean): Promise<Note> {
  const n = await api.note(id);
  return api.updateNote(id, applyEdits(n.note, { [PROCESSED]: value }), n.version);
}

export function Inbox() {
  const navigate = useNavigate();
  const [showProcessed, setShowProcessed] = createSignal(false);
  const [limit, setLimit] = createSignal(PAGE);
  const [error, setError] = createSignal<string | null>(null);
  // Notes acted on, hidden until the list loads again without them.
  const [gone, setGone] = createSignal<Set<string>>(new Set());
  const query = () => `${showProcessed() ? `from ${FLEETING}` : INBOX_QUERY} sort created desc, id desc`;
  const [page] = createResource(
    () => ({ q: query(), n: limit(), v: changes() }),
    async ({ q, n }) => {
      const p = await api.query(q, null, n);
      setGone(new Set<string>());
      return p;
    },
  );
  const rows = createMemo(() => (page.latest?.rows ?? []).filter((r) => !gone().has(r.id)));
  const hide = (id: string) => setGone((s) => new Set(s).add(id));

  async function run(id: string, act: () => Promise<void>) {
    setError(null);
    try {
      await act();
    } catch (e) {
      setError(errorText(e));
      setGone((s) => {
        const next = new Set(s);
        next.delete(id);
        return next;
      });
    }
  }

  const done = (r: Row, value: boolean) =>
    run(r.id, async () => {
      if (!showProcessed()) hide(r.id);
      await setProcessed(r.id, value);
      if (value) {
        showToast(`“${r.title}” marked done`, {
          label: "Undo",
          run: () => void run(r.id, async () => void (await setProcessed(r.id, false))),
        });
      }
    });

  const remove = (r: Row) =>
    run(r.id, async () => {
      hide(r.id);
      const n = await api.note(r.id);
      await api.deleteNote(r.id, n.version);
      showToast(`“${r.title}” moved to the trash`, {
        label: "Undo",
        run: () => void run(r.id, async () => void (await api.restoreFromTrash(r.id))),
      });
    });

  const convert = (r: Row, typeName: string) =>
    run(r.id, async () => {
      const target = typeDef(typeName);
      if (!target) throw new Error(`There is no type ${typeName}.`);
      const n = await api.note(r.id);
      const made = await api.createNote(typeName, convertDoc(n.note, typeDef(FLEETING), target));
      const title = made.note.fields.title;
      if (typeof title === "string") knowTitle(made.id, title, made.type);
      hide(r.id);
      await api.deleteNote(r.id, n.version);
      showToast(`Converted to ${target.label.toLowerCase()}`);
      navigate(`/notes/${made.id}`);
    });

  return (
    <section class="list-page inbox-page">
      <header class="note-header inbox-header">
        <h1>Inbox</h1>
        <div class="inbox-tools">
          <label class="inbox-toggle">
            <input type="checkbox" checked={showProcessed()} onChange={(e) => setShowProcessed(e.currentTarget.checked)} />
            Show processed
          </label>
          <button type="button" class="primary" onClick={openCapture}>
            New capture
          </button>
        </div>
      </header>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={page.error}>
        <p class="error" role="alert">
          {errorText(page.error)}
        </p>
      </Show>
      <Show when={page.latest}>
        <Show
          when={rows().length > 0}
          fallback={
            <p class="muted inbox-empty">
              {showProcessed()
                ? "There are no fleeting notes yet."
                : "Your inbox is empty. Capture a thought or an image with the + button or Ctrl+Shift+K, and it waits here until you process it."}
            </p>
          }
        >
          <ul class="inbox-list" aria-label="Fleeting notes">
            <For each={rows()}>
              {(r) => (
                <InboxItem
                  row={r}
                  onDone={(v) => void done(r, v)}
                  onDelete={() => void remove(r)}
                  onConvert={(t) => void convert(r, t)}
                />
              )}
            </For>
          </ul>
          <Show when={(page.latest?.total ?? 0) > limit()}>
            <button type="button" class="inbox-more" onClick={() => setLimit((n) => n + PAGE)}>
              Show more
            </button>
          </Show>
        </Show>
      </Show>
    </section>
  );
}

function InboxItem(props: { row: Row; onDone: (value: boolean) => void; onDelete: () => void; onConvert: (type: string) => void }) {
  const [note] = createResource(
    () => props.row.id,
    (id) => api.note(id).catch(() => null),
  );
  const [converting, setConverting] = createSignal(false);
  const fields = () => captureFields(typeDef(FLEETING));
  const images = () => {
    const f = fields().image;
    return f ? assetIds(props.row.fields[f.name]) : [];
  };
  const processed = () => props.row.fields[PROCESSED] === true;
  const preview = () => {
    const n = note();
    const body = fields().body;
    if (!n || !body) return "";
    return blocksText(n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === body.name)?.blocks);
  };
  const created = () => {
    const c = note()?.note.fields.created;
    return typeof c === "string" ? new Date(c) : null;
  };
  const href = () => `/notes/${props.row.id}`;
  const targets = () => types().filter((t) => t.name !== FLEETING);
  return (
    <li class="inbox-item" classList={{ processed: processed() }} data-note={props.row.id}>
      <Show when={images().length}>
        <a href={href()} class="inbox-thumbs" tabindex={-1} aria-hidden="true">
          <For each={images().slice(0, 3)}>{(id) => <img src={assetUrl(id)} alt="" loading="lazy" />}</For>
          <Show when={images().length > 3}>
            <span class="inbox-more-images">+{images().length - 3}</span>
          </Show>
        </a>
      </Show>
      <div class="inbox-main">
        <a href={href()} class="inbox-title">
          {props.row.title || "Untitled"}
        </a>
        <Show when={preview()}>
          <p class="inbox-preview">{preview()}</p>
        </Show>
        <p class="muted inbox-meta">
          <Show when={created()}>
            {(c) => <time datetime={c().toISOString()}>{c().toLocaleString()}</time>}
          </Show>
          <Show when={processed()}>
            <span class="inbox-processed-badge">Processed</span>
          </Show>
        </p>
        <Show when={converting()}>
          <div
            class="inbox-convert"
            role="group"
            aria-label="Convert to"
            ref={(el) => queueMicrotask(() => el.querySelector<HTMLButtonElement>(".chip-add")?.click())}
          >
            <span id={`convert-${props.row.id}`}>Convert to</span>
            <ChipPicker
              label="Type"
              labelledBy={`convert-${props.row.id}`}
              single
              values={[]}
              suggest={(t) => typeSuggestions(targets(), t)}
              placeholder="Find a type"
              onChange={(v) => {
                if (v[0]) props.onConvert(v[0]);
              }}
            />
            <button type="button" class="link" onClick={() => setConverting(false)}>
              Cancel
            </button>
          </div>
        </Show>
      </div>
      <div class="inbox-actions">
        <a href={href()} class="button">
          Open
        </a>
        <Show
          when={!processed()}
          fallback={
            <button type="button" onClick={() => props.onDone(false)}>
              Back to inbox
            </button>
          }
        >
          <button type="button" onClick={() => props.onDone(true)}>
            Done
          </button>
        </Show>
        <button type="button" aria-expanded={converting()} onClick={() => setConverting(!converting())}>
          Convert to…
        </button>
        <button type="button" class="danger" onClick={props.onDelete}>
          Delete
        </button>
      </div>
    </li>
  );
}
