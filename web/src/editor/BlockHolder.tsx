import { Show } from "solid-js";
import type { Block } from "../api";
import { BlockView } from "../components/Render";
import type { SolidNodeViewProps } from "./solidNodeView";

/**
 * A content block the editor holds as one node: an image, an asset, code, a terminal or a
 * diagram is edited in place by its own component; any other kind is shown read-only.
 */
export function BlockHolder(props: SolidNodeViewProps) {
  const block = () => props.node().attrs.block as Block | null;
  return (
    <div class="block-holder" classList={{ selected: props.selected() }}>
      {/* Not keyed: an edit replaces the block, and its component must stay mounted. */}
      <Show when={block()}>
        {(b) => (
          <BlockView
            block={b()}
            onChange={props.editor.isEditable ? (next) => props.updateAttributes({ block: next }) : undefined}
          />
        )}
      </Show>
    </div>
  );
}
