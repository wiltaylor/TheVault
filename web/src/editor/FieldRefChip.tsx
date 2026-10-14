import type { SolidNodeViewProps } from "./solidNodeView";

/** `{{name}}` in the template editor: a chip naming the field whose value the note shows there. */
export function FieldRefChip(props: SolidNodeViewProps) {
  const name = () => String(props.node().attrs.name ?? "");
  return (
    <span class="field-ref-chip" data-testid="field-ref" data-field-ref={name()} title={`Shows the note's ${name()}`}>
      {`{{${name()}}}`}
    </span>
  );
}
