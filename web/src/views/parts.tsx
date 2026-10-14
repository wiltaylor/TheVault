// Pieces every query view kind renders a result note with.

import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { NoteDoc, Row, Template } from "../api";
import { FieldRefs } from "../components/fieldRefs";
import { refText } from "../components/refText";
import { BlockView, Blocks, formatValue } from "../components/Render";
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
                      <div class="template-block">
                        <BlockView block={item.block} />
                      </div>
                    </FieldRefs.Provider>
                  </ThisNote.Provider>
                );
              }
              if (item.slot === "title") return null;
              const t = type();
              const f = t ? slotField(t, item.slot) : undefined;
              if (!f) return null;
              const label = item.label ?? f.label;
              return (
                <div class="query-slot" data-field={f.name}>
                  <span class="query-slot-label">{label}</span>
                  <span class="query-slot-value">
                    <Show
                      when={f.compound}
                      fallback={formatValue(props.doc ? props.doc.fields[f.name] : props.row.fields[f.name])}
                    >
                      <Show when={props.doc}>{(doc) => <Blocks blocks={compoundBlock(doc(), f)?.blocks ?? []} />}</Show>
                    </Show>
                  </span>
                </div>
              );
            }}
          </For>
        </section>
      )}
    </For>
  );
}
