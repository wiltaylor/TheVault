import { mentionText } from "./titles";
import type { SolidNodeViewProps } from "./solidNodeView";

/** A mention inside the editor: its label, or its target's current title. */
export function MentionChip(props: SolidNodeViewProps) {
  const id = () => String(props.node().attrs.id ?? "");
  const label = () => (props.node().attrs.label as string | null) ?? null;
  return (
    <span class="mention" data-testid="mention" data-id={id()} title={id()}>
      {mentionText(id(), label())}
    </span>
  );
}
