// A `card` content block (docs/vault-format.md, "card"): another note shown as a card, with its
// type's `card` template, that opens the note when clicked. A card of a note that is missing shows
// so and stays, as a mention of one does.

import { Show, createContext, createResource, createSignal, onMount, useContext } from "solid-js";
import { type Block, type NoteDoc, type Row, type TypeDef, api } from "../api";
import { changes } from "../changes";
import { Combobox } from "../components/Combobox";
import { findNotes } from "../components/FieldInput";
import { TypeIcon } from "../components/Icon";
import { typeDef } from "../session";
import { TemplateBody } from "../views/parts";
import { followLink, wantsNewTab } from "../workspace/events";
import { text, withItem } from "./fields";
import { BinIcon } from "./MediaBlocks";

/** Inside a card: a card in its template shows only its title, so cards never nest without end. */
const InCard = createContext(false);

/** The type's own `card` template. A card block does not fall back to `default`. */
export function cardBlockTemplate(type: TypeDef | undefined) {
  return type?.templates.find((t) => t.name === "card");
}

/** The parts of a card a click on which is theirs, not the card's. */
const OWN_CLICKS = "a[href], button, input, select, textarea, [data-node-view-interactive]";

/**
 * A `card` block. With `onChange`, it is being edited: a card with no note yet is a search for
 * one, and Change and Remove sit over its corner. A click on the card, anywhere but its own
 * buttons and links, opens the note; Ctrl/Cmd+click or a middle-click opens it in the background.
 */
export function NoteCardBlock(props: { block: Block; onChange?: (block: Block) => void; selected?: boolean; onRemove?: () => void }) {
  const id = () => text(props.block, "note");
  const [changing, setChanging] = createSignal(false);
  const choose = (note: string) => {
    setChanging(false);
    props.onChange?.(withItem(props.block, "note", note));
  };
  return (
    <Show
      when={props.onChange && (!id() || changing())}
      fallback={
        <div class="note-card-block" classList={{ selected: !!props.selected, editing: !!props.onChange }} data-testid="note-card-block">
          <Show when={id()} fallback={<MissingCard id="" />}>
            {(i) => <NoteCard id={i()} />}
          </Show>
          <Show when={props.onChange}>
            <div class="block-image-tools note-card-tools" role="toolbar" aria-label="Card">
              <button type="button" aria-label="Change note" title="Change note" onMouseDown={(e) => e.preventDefault()} onClick={() => setChanging(true)}>
                <SwapIcon />
              </button>
              <Show when={props.onRemove}>
                {(remove) => (
                  <button type="button" aria-label="Remove card" title="Remove card" onMouseDown={(e) => e.preventDefault()} onClick={() => remove()()}>
                    <BinIcon />
                  </button>
                )}
              </Show>
            </div>
          </Show>
        </div>
      }
    >
      <NotePicker
        onChoose={choose}
        onCancel={() => {
          // Escape on a new card takes it out again; on a card being changed, keeps its note.
          if (id()) setChanging(false);
          else props.onRemove?.();
        }}
        onBlur={() => {
          if (id()) setChanging(false);
        }}
      />
    </Show>
  );
}

/** The search a card's note is chosen with: the reference field's note search. */
function NotePicker(props: { onChoose: (id: string) => void; onCancel: () => void; onBlur: () => void }) {
  let box!: HTMLDivElement;
  // Focus once the editor has placed the block and taken focus itself.
  onMount(() => queueMicrotask(() => box.querySelector("input")?.focus()));
  return (
    <div class="note-card-picker" data-testid="note-card-picker" ref={box}>
      <Combobox
        label="Note to show as a card"
        placeholder="Find a note to show as a card"
        suggest={async (q) => (await findNotes("*", q)).map((r: Row) => ({ value: r.id, text: r.title || "Untitled", icon: typeDef(r.type)?.icon ?? null, hint: typeDef(r.type)?.label ?? r.type }))}
        choose={(s) => {
          if (s) props.onChoose(s.value);
        }}
        onDone={(how) => (how === "escape" ? props.onCancel() : props.onBlur())}
      />
    </div>
  );
}

/** The note `id` as a card: its type icon and title, then its `card` template, if the type has one. */
function NoteCard(props: { id: string }) {
  const nested = useContext(InCard);
  const [note] = createResource(
    () => [props.id, changes()] as const,
    ([i]) => api.note(i),
  );
  return (
    <Show when={!note.error} fallback={<MissingCard id={props.id} />}>
      <Show when={note.latest} fallback={<div class="note-card muted" data-testid="note-card">Loading…</div>}>
        {(n) => {
          const type = () => typeDef(n().type);
          const row = (): Row => ({ id: n().id, type: n().type, title: titleOf(n().note), fields: n().note.fields });
          const template = () => (nested ? undefined : cardBlockTemplate(type()));
          return (
            <article
              class="note-card"
              classList={{ archived: n().note.fields.archived === true }}
              data-testid="note-card"
              data-id={n().id}
              data-type={n().type}
              style={type()?.colour ? { "--note-card": `var(--palette-${type()!.colour})` } : undefined}
              onClick={(e) => open(e, n().id)}
              onAuxClick={(e) => e.button === 1 && open(e, n().id)}
            >
              <a class="note-card-title" href={`/notes/${n().id}`} draggable={false} data-testid="note-card-title">
                <TypeIcon name={type()?.icon} size={16} />
                <span>{row().title || "Untitled"}</span>
              </a>
              <Show when={template()}>
                {(t) => (
                  <InCard.Provider value={true}>
                    <div class="note-card-body">
                      <TemplateBody row={row()} template={t()} doc={n().note} />
                    </div>
                  </InCard.Provider>
                )}
              </Show>
            </article>
          );
        }}
      </Show>
    </Show>
  );
}

/** A card whose note does not exist, or could not be read. */
function MissingCard(props: { id: string }) {
  return (
    <div class="note-card missing" data-testid="note-card" data-missing="true" title={props.id || undefined}>
      <span class="note-card-title muted">Missing note</span>
    </div>
  );
}

function titleOf(doc: NoteDoc): string {
  const t = doc.fields.title;
  return typeof t === "string" ? t : "";
}

/** Open note `id` for a click on its card, unless the click was on something of its own. */
function open(e: MouseEvent, id: string) {
  if (e.defaultPrevented || e.altKey || e.shiftKey || e.detail > 1) return;
  if (e.target instanceof Element && e.target.closest(OWN_CLICKS)) return;
  const card = e.currentTarget as Element;
  // A drag that selected text in the card is not a click on it.
  const sel = window.getSelection();
  if (sel && !sel.isCollapsed && sel.anchorNode && card.contains(sel.anchorNode)) return;
  e.preventDefault();
  followLink(`/notes/${id}`, card, wantsNewTab(e));
}

function SwapIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="m16 3 4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16" />
    </svg>
  );
}
