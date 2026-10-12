import { Index, Show } from "solid-js";
import type { Block } from "../api";
import {
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

/** Text for an optional string item: empty text removes it. */
const optional = (v: string) => (v.trim() === "" ? undefined : v);

/**
 * The settings of a query view block in the editor: its heading, its query, and its saved views
 * (D8). Every change hands `onChange` the whole new block, which the editor stores in the note.
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
      <label>
        Query
        <textarea
          rows={2}
          spellcheck={false}
          value={def().query}
          onInput={(e) => set(withItem(props.block, "query", e.currentTarget.value))}
        />
      </label>
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
                <input
                  type="text"
                  value={view().template ?? ""}
                  placeholder={view().kind === "details" ? "detail" : "card"}
                  onInput={(e) => set(setViewItem(props.block, i, "template", optional(e.currentTarget.value)))}
                />
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
