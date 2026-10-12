import { Show } from "solid-js";
import type { Block } from "../api";
import { BlockView } from "../components/Render";
import { QueryViewSettings } from "./QueryViewSettings";
import type { SolidNodeViewProps } from "./solidNodeView";

/**
 * A content block the editor keeps as it is and shows read-only, except a query view, whose
 * settings are edited in place.
 */
export function BlockHolder(props: SolidNodeViewProps) {
  const block = () => props.node().attrs.block as Block | null;
  return (
    <div class="block-holder" classList={{ selected: props.selected() }}>
      <Show when={block()}>
        {(b) => (
          <Show when={b().kind === "query_view"} fallback={<BlockView block={b()} />}>
            <QueryViewSettings
              block={b()}
              onChange={(next) => props.updateAttributes({ block: next })}
              onRemove={() => props.replaceWith()}
            />
          </Show>
        )}
      </Show>
    </div>
  );
}
