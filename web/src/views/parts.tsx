// Pieces every query view kind renders a result note with.

import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { NoteDoc, Row, Template } from "../api";
import { FieldRefs } from "../components/fieldRefs";
import { refText } from "../components/refText";
import { BlockView, Blocks, ColumnsView, FieldValue } from "../components/Render";
import { slotOfBlock } from "../blocks/columns";
import { compoundBlock, fieldValue, labelFields, slotField, whenNote } from "../notes";
import { shownTabs, whenHolds } from "../when";
import { type ControlBinding, ControlBindingContext } from "../controls/binding";
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
  if (!f || labelFields(t!).has(f.name)) return null;
  const label = props.label ?? f.label;
  const fields = () => (props.doc ? props.doc.fields : props.row.fields);
  return (
    <div class="query-slot" data-field={f.name}>
      <span class="query-slot-label">{label}</span>
      <span class="query-slot-value">
        <Show when={f.compound} fallback={<FieldValue field={f} value={fields()[f.name]} fields={fields()} size="card" />}>
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
  // Controls in the template show this note's values, read-only: a result is edited on its page.
  const controls: ControlBinding = {
    type,
    value: (f) => (props.doc ? fieldValue(props.doc, f) : f.name === "title" ? props.row.title : props.row.fields[f.name]),
    reason: "Open the note to change this.",
  };
  // The tabs whose `when` holds for this note; a card, without tab titles, shows the first.
  const tabs = () => {
    const t = type();
    const all = props.template?.tabs ?? [];
    const doc = props.doc ?? { fields: { ...props.row.fields, title: props.row.title }, blocks: [] };
    return t ? shownTabs(all, whenNote(t, props.row.id, doc), !props.tabTitles) : all.slice(0, props.tabTitles ? undefined : 1);
  };
  // A slot's `when`, read as a tab's, on this note.
  const slotShows = (when: unknown) => {
    const t = type();
    if (typeof when !== "string" || !when || !t) return true;
    const doc = props.doc ?? { fields: { ...props.row.fields, title: props.row.title }, blocks: [] };
    return whenHolds(when, whenNote(t, props.row.id, doc));
  };
  // On a card, an image from a field that leads the template is the card's cover: drawn first,
  // above the title (styles.css, `.card-cover`).
  const cover = () => {
    if (props.tabTitles) return undefined;
    const first = tabs()[0]?.items[0];
    return first && !("slot" in first) && first.block.kind === "image" && typeof first.block.fields?.field === "string"
      ? first.block
      : undefined;
  };
  return (
    <ControlBindingContext.Provider value={controls}>
    <Show when={cover()}>
      {(block) => (
        <FieldRefs.Provider value={refs}>
          <div class="card-cover" data-testid="card-cover">
            <BlockView block={block()} />
          </div>
        </FieldRefs.Provider>
      )}
    </Show>
    <For each={tabs()}>
      {(tab) => (
        <section class="template-tab">
          <Show when={props.tabTitles && tabs().length > 1}>
            <h4>{tab.title}</h4>
          </Show>
          <For each={tab.items}>
            {(item) => {
              if (!("slot" in item) && item.block === cover()) return null;
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
                              if (!slotShows(b.fields?.when)) return null;
                              return <SlotValue row={props.row} doc={props.doc} slot={slot} label={typeof label === "string" ? label : null} />;
                            }}
                          />
                        </div>
                      </Show>
                    </FieldRefs.Provider>
                  </ThisNote.Provider>
                );
              }
              if (!slotShows(item.when)) return null;
              return <SlotValue row={props.row} doc={props.doc} slot={item.slot} label={item.label} />;
            }}
          </For>
        </section>
      )}
    </For>
    </ControlBindingContext.Provider>
  );
}
