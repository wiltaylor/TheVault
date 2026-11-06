// The inbox: what waits to be processed, newest first by each item's own time: fleeting notes,
// incoming mail, incoming messages and missed calls (src/inbox/sources.ts). Filter chips narrow
// it to one source. Every row can be opened or marked done. A fleeting note can also be converted
// into a note of another type; every row but an email can be moved to the trash.

import { useNavigate } from "@solidjs/router";
import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import { type Note, type Row, api, assetUrl } from "../api";
import { FLEETING, PROCESSED, assetIds, blocksText, captureFields, convertDoc } from "../capture/model";
import { openCapture, refreshInboxCount } from "../capture/store";
import { changes } from "../changes";
import { ChipPicker, typeSuggestions } from "../components/ChipPicker";
import { showToast } from "../components/Toasts";
import { knowTitle } from "../editor/titles";
import { appInfo } from "../inApp";
import { CatchUpDialog } from "../inbox/CatchUpDialog";
import { loadInbox } from "../inbox/load";
import { FILTERS, type Filter, type Item } from "../inbox/sources";
import { applyEdits } from "../notes";
import { typeDef, types } from "../session";
import { errorText } from "./TypeEditor";

const PAGE = 50;

/** What a row of each source is called beside its time; a capture needs no word. */
const KINDS: Record<Item["source"]["key"], string | null> = {
  captures: null,
  mail: "Mail",
  messages: "Message",
  calls: "Missed call",
};

/** Mark note `id` processed or not, over its current version. */
async function setProcessed(id: string, value: boolean): Promise<Note> {
  const n = await api.note(id);
  return api.updateNote(id, applyEdits(n.note, { [PROCESSED]: value }), n.version);
}

export function Inbox() {
  const navigate = useNavigate();
  const [showProcessed, setShowProcessed] = createSignal(false);
  const [filter, setFilter] = createSignal<Filter>("all");
  // The one-time catch-up runs on the home server only, so the apps do not offer it (TV-30).
  const [app] = createResource(appInfo);
  const [catchingUp, setCatchingUp] = createSignal(false);
  const [reload, setReload] = createSignal(0);
  const [limit, setLimit] = createSignal(PAGE);
  const [error, setError] = createSignal<string | null>(null);
  // Notes acted on, hidden until the list loads again without them.
  const [gone, setGone] = createSignal<Set<string>>(new Set());
  // A captures row needs its image field for the thumbnails.
  const extra = () => {
    const image = captureFields(typeDef(FLEETING)).image;
    return image ? [image.name] : [];
  };
  const [page] = createResource(
    () => ({ f: filter(), all: showProcessed(), n: limit(), extra: extra(), v: changes(), r: reload() }),
    async ({ f, all, n, extra }) => {
      const p = await loadInbox(f, all, n, extra);
      setGone(new Set<string>());
      return p;
    },
  );
  const items = createMemo(() => (page.latest?.items ?? []).filter((i) => !gone().has(i.row.id)));
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
          <div class="inbox-chips" role="group" aria-label="Show">
            <For each={FILTERS}>
              {(f) => (
                <button
                  type="button"
                  class="inbox-chip"
                  aria-pressed={filter() === f.key}
                  onClick={() => {
                    setFilter(f.key);
                    setLimit(PAGE);
                  }}
                >
                  {f.label}
                </button>
              )}
            </For>
          </div>
          <label class="inbox-toggle">
            <input type="checkbox" checked={showProcessed()} onChange={(e) => setShowProcessed(e.currentTarget.checked)} />
            Show processed
          </label>
          <Show when={app.state === "ready" && app() === null}>
            <button type="button" onClick={() => setCatchingUp(true)}>
              Mark older processed…
            </button>
          </Show>
          <button type="button" class="primary" onClick={openCapture}>
            New capture
          </button>
        </div>
      </header>
      <Show when={catchingUp()}>
        <CatchUpDialog
          onCancel={() => setCatchingUp(false)}
          onDone={(changed) => {
            setCatchingUp(false);
            setReload((n) => n + 1);
            void refreshInboxCount();
            showToast(changed === 1 ? "Marked 1 item processed" : `Marked ${changed} items processed`);
          }}
        />
      </Show>
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
          when={items().length > 0}
          fallback={
            <p class="muted inbox-empty">
              {showProcessed()
                ? "There is nothing here yet."
                : "Your inbox is empty. Captures, incoming mail and messages and missed calls wait here until you process them. Capture a thought or an image with the + button or Ctrl+Shift+K."}
            </p>
          }
        >
          <ul class="inbox-list" aria-label="Inbox">
            <For each={items()}>
              {(i) => (
                <InboxItem
                  item={i}
                  onDone={(v) => void done(i.row, v)}
                  onDelete={() => void remove(i.row)}
                  onConvert={(t) => void convert(i.row, t)}
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

function InboxItem(props: { item: Item; onDone: (value: boolean) => void; onDelete: () => void; onConvert: (type: string) => void }) {
  const row = () => props.item.row;
  const capture = () => props.item.source.key === "captures";
  // Only a capture's preview needs the whole note: its body is a content block.
  const [note] = createResource(
    () => (capture() ? row().id : null),
    (id) => api.note(id).catch(() => null),
  );
  const [converting, setConverting] = createSignal(false);
  const fields = () => captureFields(typeDef(FLEETING));
  const images = () => {
    const f = fields().image;
    return capture() && f ? assetIds(row().fields[f.name]) : [];
  };
  const processed = () => row().fields[PROCESSED] === true;
  const preview = () => {
    const n = note();
    const body = fields().body;
    if (!n || !body) return "";
    return blocksText(n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === body.name)?.blocks);
  };
  const when = () => (props.item.time === null ? null : new Date(props.item.time));
  // Who a mail or message is from, as the sync or the phone wrote it.
  const from = () => {
    const f = row().fields.from_raw;
    return typeof f === "string" && f !== "" ? f : null;
  };
  const kind = () => KINDS[props.item.source.key];
  const snippet = () => {
    const s = row().fields.snippet;
    return typeof s === "string" ? s : "";
  };
  const href = () => `/notes/${row().id}`;
  const targets = () => types().filter((t) => t.name !== FLEETING);
  return (
    <li class="inbox-item" classList={{ processed: processed() }} data-note={row().id} data-source={props.item.source.key}>
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
          {row().title || "Untitled"}
        </a>
        <Show when={preview() || snippet()}>
          <p class="inbox-preview">{preview() || snippet()}</p>
        </Show>
        <p class="muted inbox-meta">
          <Show when={kind()}>{(k) => <span class="inbox-kind">{k()}</span>}</Show>
          <Show when={from()}>{(f) => <span class="inbox-from">{f()}</span>}</Show>
          <Show when={when()}>
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
            <span id={`convert-${row().id}`}>Convert to</span>
            <ChipPicker
              label="Type"
              labelledBy={`convert-${row().id}`}
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
        <Show when={capture()}>
          <button type="button" aria-expanded={converting()} onClick={() => setConverting(!converting())}>
            Convert to…
          </button>
        </Show>
        {/* The trash removes only the vault's copy of a mail, so an email row does not offer it. */}
        <Show when={props.item.source.key !== "mail"}>
          <button type="button" class="danger" onClick={props.onDelete}>
            Delete
          </button>
        </Show>
      </div>
    </li>
  );
}
