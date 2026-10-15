import { mentionText, noteArchived } from "./titles";
import type { SolidNodeViewProps } from "./solidNodeView";

/** A mention inside the editor: its label, or its target's current title, linking to the note. */
export function MentionChip(props: SolidNodeViewProps) {
  const id = () => String(props.node().attrs.id ?? "");
  const label = () => (props.node().attrs.label as string | null) ?? null;
  return (
    <a
      class="mention"
      classList={{ archived: noteArchived(id()) === true }}
      href={`/notes/${id()}`}
      draggable={false}
      data-testid="mention"
      data-id={id()}
      title={noteArchived(id()) ? "Archived" : id()}
    >
      {mentionText(id(), label())}
    </a>
  );
}
