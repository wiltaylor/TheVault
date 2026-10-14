import { Index, Show } from "solid-js";
import type { Block } from "../api";
import {
  CALENDAR_MODES,
  IDENT,
  VIEW_KINDS,
  addView,
  kindUses,
  readQueryView,
  readView,
  removeView,
  renameView,
  setDefaultView,
  setViewItem,
  setViewKind,
  viewBlocks,
  withItem,
} from "../views/model";
import { QueryEditor } from "./QueryEditor";

/** Text for an optional string item: empty text removes it. */
const optional = (v: string) => (v.trim() === "" ? undefined : v);

/**
 * The settings of a query view block in the editor: its heading, its query, as text or in the
 * visual builder, and its saved views (D8). Every change hands `onChange` the whole new block, which the editor stores in the note.
 * Lists use `Index`, so editing a view keeps its inputs, and focus, in place.
 */
export function QueryViewSettings(props: { block: Block; onChange: (block: Block) => void; onRemove: () => void }) {
  const def = () => readQueryView(props.block);
  const views = () => viewBlocks(props.block).map(readView);
  const set = (next: Block) => props.onChange(next);

  return (
    <fieldset class="query-view-settings" aria-label="Query view settings" role="group">
      <legend>Query view</legend>
      <label>
        Heading
        <input
          type="text"
          value={def().title ?? ""}
          onInput={(e) => set(withItem(props.block, "title", optional(e.currentTarget.value)))}
        />
      </label>
      <QueryEditor
        value={def().query}
        onChange={(text) => set(withItem(props.block, "query", text))}
      />
      <Index each={views()}>
        {(view, i) => (
          <div class="view-settings" data-testid="view-settings">
            <label>
              Name
              <input
                type="text"
                value={view().name}
                aria-invalid={!IDENT.test(view().name)}
                onInput={(e) => set(renameView(props.block, i, e.currentTarget.value))}
              />
            </label>
            <label>
              Kind
              <select value={view().kind} onChange={(e) => set(setViewKind(props.block, i, e.currentTarget.value))}>
                <Index each={VIEW_KINDS}>{(k) => <option value={k()}>{k()}</option>}</Index>
              </select>
            </label>
            <label>
              Tab label
              <input
                type="text"
                value={viewBlocks(props.block)[i]?.fields?.title === undefined ? "" : view().title}
                placeholder={view().name}
                onInput={(e) => set(setViewItem(props.block, i, "title", optional(e.currentTarget.value)))}
              />
            </label>
            <Show when={kindUses(view().kind, "columns")}>
              <label>
                Columns
                <input
                  type="text"
                  value={view().columns.join(", ")}
                  placeholder="title"
                  onChange={(e) => {
                    const cols = e.currentTarget.value
                      .split(",")
                      .map((c) => c.trim())
                      .filter(Boolean);
                    set(setViewItem(props.block, i, "columns", cols.length ? cols : undefined));
                  }}
                />
              </label>
            </Show>
            <Show when={kindUses(view().kind, "template")}>
              <label>
                Template
                <select
                  onChange={(e) => set(setViewItem(props.block, i, "template", optional(e.currentTarget.value)))}
                >
                  <option value="" selected={!view().template}>
                    {view().kind === "details" ? "Default (default)" : "Default (card)"}
                  </option>
                  <option value="default" selected={view().template === "default"}>
                    default
                  </option>
                  <option value="card" selected={view().template === "card"}>
                    card
                  </option>
                </select>
              </label>
            </Show>
            <Show when={kindUses(view().kind, "location")}>
              <label>
                Location field
                <input
                  type="text"
                  value={view().location ?? ""}
                  aria-invalid={!IDENT.test(view().location ?? "")}
                  onInput={(e) => set(setViewItem(props.block, i, "location", optional(e.currentTarget.value)))}
                />
              </label>
            </Show>
            <Show when={kindUses(view().kind, "group")}>
              <label>
                Group by
                <input
                  type="text"
                  value={view().group ?? ""}
                  placeholder="a single-term taxonomy field"
                  onInput={(e) => set(setViewItem(props.block, i, "group", optional(e.currentTarget.value)))}
                />
              </label>
            </Show>
            <Show when={kindUses(view().kind, "date")}>
              <label>
                Date field
                <input
                  type="text"
                  value={view().date ?? ""}
                  placeholder="a date, datetime or date range field"
                  onInput={(e) => set(setViewItem(props.block, i, "date", optional(e.currentTarget.value)))}
                />
              </label>
            </Show>
            <Show when={kindUses(view().kind, "mode")}>
              <label>
                Mode
                <select
                  value={view().mode}
                  onChange={(e) =>
                    set(setViewItem(props.block, i, "mode", e.currentTarget.value === "month" ? undefined : e.currentTarget.value))
                  }
                >
                  <Index each={CALENDAR_MODES}>{(m) => <option value={m()}>{m()}</option>}</Index>
                </select>
              </label>
            </Show>
            <Show when={views().length > 1}>
              <button type="button" onClick={() => set(removeView(props.block, i))}>
                Remove view
              </button>
            </Show>
          </div>
        )}
      </Index>
      <div class="query-view-settings-actions">
        <button type="button" onClick={() => set(addView(props.block))}>
          Add view
        </button>
        <Show when={views().length > 1}>
          <label>
            Show first
            <select
              value={def().first?.name ?? ""}
              onChange={(e) => set(setDefaultView(props.block, e.currentTarget.value))}
            >
              <Index each={views()}>{(v) => <option value={v().name}>{v().name}</option>}</Index>
            </select>
          </label>
        </Show>
        <button type="button" onClick={() => props.onRemove()}>
          Remove query view
        </button>
      </div>
    </fieldset>
  );
}
