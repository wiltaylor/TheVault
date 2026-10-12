import { Show } from "solid-js";
import type { Block } from "../api";
import { BlockView } from "../components/Render";
import { QueryViewSettings } from "./QueryViewSettings";
import type { SolidNodeViewProps } from "./solidNodeView";

/**
 * A content block the editor holds as one node: an image, an asset, code, a terminal, a
 * diagram or a drawing is edited in place by its own component, and a query view by its
 * settings; any other kind is shown read-only.
 */
export function BlockHolder(props: SolidNodeViewProps) {
  const block = () => props.node().attrs.block as Block | null;
  const onChange = () => (props.editor.isEditable ? (next: Block) => props.updateAttributes({ block: next }) : undefined);
  return (
    <div class="block-holder" classList={{ selected: props.selected() }}>
      {/* Not keyed: an edit replaces the block, and its component must stay mounted. */}
      <Show when={block()}>
        {(b) => (
          <Show
            when={b().kind === "query_view" && onChange()}
            fallback={<BlockView block={b()} onChange={onChange()} />}
          >
            {(change) => (
              <QueryViewSettings block={b()} onChange={change()} onRemove={() => props.replaceWith()} />
            )}
          </Show>
        )}
      </Show>
    </div>
  );
}
