// Pieces every query view kind renders a result note with.

import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { NoteDoc, Row, Template } from "../api";
import { FieldRefs } from "../components/fieldRefs";
import { refText } from "../components/refText";
import { BlockView, Blocks, ColumnsView, formatValue } from "../components/Render";
import { slotOfBlock } from "../blocks/columns";
import { compoundBlock, slotField } from "../notes";
import { typeDef } from "../session";
import { ThisNote } from "./thisNote";

export function NoteLink(props: { row: Row }) {
  return (
    <A class="query-title" href={`/notes/${props.row.id}`} data-testid="query-title">
      {props.row.title || "Untitled"}
    </A>
  );
}

/** One slot of a template, for one result note: the field's label and value, read-only. */
function SlotValue(props: { row: Row; doc?: NoteDoc; slot: string; label: string | null }) {
  if (props.slot === "title") return null;
  const t = typeDef(props.row.type);
  const f = t ? slotField(t, props.slot) : undefined;
  if (!f) return null;
  const label = props.label ?? f.label;
  return (
    <div class="query-slot" data-field={f.name}>
      <span class="query-slot-label">{label}</span>
      <span class="query-slot-value">
        <Show when={f.compound} fallback={formatValue(props.doc ? props.doc.fields[f.name] : props.row.fields[f.name])}>
          <Show when={props.doc}>{(doc) => <Blocks blocks={compoundBlock(doc(), f)?.blocks ?? []} />}</Show>
        </Show>
      </span>
    </div>
  );
}

/**
 * The slots and blocks of a template for one result note, below its title. Values come from
 * `doc` when the full note is loaded, else from the row. Blocks see the result note as `this`.
 */
export function TemplateBody(props: { row: Row; template: Template | undefined; doc?: NoteDoc; tabTitles?: boolean }) {
  const type = () => typeDef(props.row.type);
  // `{{name}}` in the template's fixed blocks shows this note's values.
  const refs = {
    text: (name: string) =>
      refText(type(), props.doc ?? { fields: { ...props.row.fields, title: props.row.title }, blocks: [] }, name),
  };
  return (
    <For each={props.template?.tabs ?? []}>
      {(tab) => (
        <section class="template-tab">
          <Show when={props.tabTitles && (props.template?.tabs.length ?? 0) > 1}>
            <h4>{tab.title}</h4>
          </Show>
          <For each={tab.items}>
            {(item) => {
              if (!("slot" in item)) {
                return (
                  <ThisNote.Provider value={props.row.id}>
                    <FieldRefs.Provider value={refs}>
                      <Show when={item.block.kind === "columns"} fallback={<div class="template-block"><BlockView block={item.block} /></div>}>
                        <div class="template-columns">
                          <ColumnsView
                            block={item.block}
                            child={(b) => {
                              const slot = slotOfBlock(b);
                              if (slot === null) return <div class="template-block"><BlockView block={b} /></div>;
                              const label = b.fields?.label;
                              return <SlotValue row={props.row} doc={props.doc} slot={slot} label={typeof label === "string" ? label : null} />;
                            }}
                          />
                        </div>
                      </Show>
                    </FieldRefs.Provider>
                  </ThisNote.Provider>
                );
              }
              return <SlotValue row={props.row} doc={props.doc} slot={item.slot} label={item.label} />;
            }}
          </For>
        </section>
      )}
    </For>
  );
}
