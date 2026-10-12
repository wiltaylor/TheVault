import type { Block } from "../api";
import { BlockView } from "../components/Render";
import type { SolidNodeViewProps } from "./solidNodeView";

/** A content block the editor keeps as it is and shows read-only. */
export function BlockHolder(props: SolidNodeViewProps) {
  const block = () => props.node().attrs.block as Block | null;
  return (
    <div class="block-holder" classList={{ selected: props.selected() }}>
      {block() ? <BlockView block={block() as Block} /> : null}
    </div>
  );
}
