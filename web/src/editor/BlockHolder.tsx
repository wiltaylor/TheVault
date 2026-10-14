import { Show, createSignal, untrack } from "solid-js";
import type { Block } from "../api";
import { BlockView } from "../components/Render";
import { QueryViewSettings } from "./QueryViewSettings";
import { ControlHolder } from "../controls/ControlHolder";
import type { SolidNodeViewProps } from "./solidNodeView";

/** Blocks inserted in this session, which open ready to set up. */
const inserted = new WeakSet<Block>();

/** Mark `block` as just inserted, so a query view opens on its settings. */
export function fresh(block: Block): Block {
  inserted.add(block);
  return block;
}

/**
 * A content block the editor holds as one node: an image, an asset, code, a
 * diagram, a drawing or a canvas is edited in place by its own component. A query view shows its
 * results, and its settings on demand; any other kind is shown read-only.
 */
export function BlockHolder(props: SolidNodeViewProps) {
  const block = () => props.node().attrs.block as Block | null;
  const editable = () => props.editor.isEditable;
  const onChange = () => (editable() ? (next: Block) => props.updateAttributes({ block: next }) : undefined);
  // Read once, untracked: the node view's render must not depend on the node.
  const first = untrack(block);
  const [settings, setSettings] = createSignal(first !== null && inserted.has(first));
  return (
    <div class="block-holder" classList={{ selected: props.selected() }}>
      {/* Not keyed: an edit replaces the block, and its component must stay mounted. */}
      <Show when={block()}>
        {(b) => (
          <Show
            when={b().kind === "query_view" && onChange()}
            fallback={
              <Show when={b().kind === "control"} fallback={<BlockView block={b()} onChange={onChange()} />}>
                <ControlHolder
                  block={b()}
                  selected={props.selected()}
                  fresh={first !== null && inserted.has(first)}
                  onChange={onChange()}
                  onRemove={editable() ? () => props.replaceWith() : undefined}
                />
              </Show>
            }
          >
            {(change) => (
              <div class="query-view-holder">
                <div class="block-holder-bar">
                  <button
                    type="button"
                    aria-label={settings() ? "Close query view settings" : "Edit query view"}
                    aria-pressed={settings()}
                    onClick={() => setSettings(!settings())}
                  >
                    {settings() ? "Done" : "Edit query"}
                  </button>
                </div>
                <Show when={settings()} fallback={<BlockView block={b()} />}>
                  <QueryViewSettings block={b()} onChange={change()} onRemove={() => props.replaceWith()} />
                </Show>
              </div>
            )}
          </Show>
        )}
      </Show>
    </div>
  );
}
