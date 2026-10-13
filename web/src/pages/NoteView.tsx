import { A, useNavigate, useParams, useSearchParams } from "@solidjs/router";
import { For, Show, createEffect, createSignal, on } from "solid-js";
import { ApiError, type Block, type FieldDef, type Json, type Note, type NoteDoc, type Tab, api } from "../api";
import { ConflictView } from "../components/ConflictView";
import { FieldInput, hasInput } from "../components/FieldInput";
import { BlockView, Blocks, formatValue } from "../components/Render";
import { ContentEditor } from "../editor/ContentEditor";
import { knowTitle, noteTitle } from "../editor/titles";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { InkSurface } from "../ink/InkSurface";
import {
  applyBlocks,
  applyCompound,
  applyDict,
  applyEdits,
  compoundBlock,
  fieldValue,
  pageTemplate,
  slotField,
} from "../notes";
import { typeDef } from "../session";
import { ThisNote } from "../views/thisNote";
import { isPinned, togglePin } from "../pins";

interface Conflict {
  mine: NoteDoc;
  stored: Note;
}

function message(e: unknown): string {
  if (e instanceof ApiError) {
    const issues = e.body.issues as { at: string; message: string }[] | undefined;
    if (issues?.length) return issues.map((i) => `${i.at}: ${i.message}`).join("; ");
  }
  return e instanceof Error ? e.message : String(e);
}

/** The form for a new note of the type named in `?type=`: its default template, empty (D14). */
export function NewNote() {
  const [search] = useSearchParams();
  const type = () => (typeof search.type === "string" ? search.type : "");
  return (
    <Show when={typeDef(type())} keyed fallback={<p class="error">There is no type {type()}.</p>}>
      {(t) => <NoteView newType={t.name} />}
    </Show>
  );
}

/** A draft of a note of `type` that has not been created yet. */
function draftNote(type: string): Note {
  return { id: "", type, version: "", wcl: "", note: { fields: {}, blocks: [] } };
}

/**
 * A note laid out by its type's default template, a tab per template tab (D14). With `newType`,
 * an unsaved note of that type, open for editing, which Save creates.
 */
export function NoteView(props: { newType?: string }) {
  const params = useParams();
  const navigate = useNavigate();
  const [note, setNote] = createSignal<Note | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [tab, setTab] = createSignal(0);
  const [editing, setEditing] = createSignal(false);
  // Field values edited so far, by field name. `undefined` clears a field.
  const [edits, setEdits] = createSignal<Record<string, Json | undefined>>({});
  // Content fields edited in the block editor, by field name.
  let blockEdits = new Map<string, { field: FieldDef; blocks: Block[] }>();
  // Ink and canvas fields changed, by field name.
  const [inkEdits, setInkEdits] = createSignal<Record<string, { field: FieldDef; block: Block }>>({});
  const [saving, setSaving] = createSignal(false);
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [conflict, setConflict] = createSignal<Conflict | null>(null);

  createEffect(
    on(
      () => params.id,
      async (id) => {
        setNote(null);
        setLoadError(null);
        setTab(0);
        setEditing(false);
        setConflict(null);
        if (props.newType) {
          setNote(draftNote(props.newType));
          startEditing();
          return;
        }
        try {
          setNote(await api.note(id ?? ""));
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? "This note does not exist." : message(e));
        }
      },
    ),
  );

  const type = () => {
    const n = note();
    return n ? typeDef(n.type) : undefined;
  };
  const tabs = (): Tab[] => {
    const t = type();
    return (t && pageTemplate(t)?.tabs) ?? [];
  };

  function startEditing() {
    setEdits({});
    blockEdits = new Map();
    setInkEdits({});
    setSaveError(null);
    setEditing(true);
  }

  function draftValue(f: FieldDef): Json | undefined {
    const n = note();
    if (f.name in edits()) return edits()[f.name];
    // A new note shows each field's default, which the server fills in when the field is not sent.
    if (props.newType) return f.default;
    return n ? fieldValue(n.note, f) : undefined;
  }

  async function save(doc: NoteDoc, base: string) {
    const n = note();
    if (!n) return;
    setSaving(true);
    setSaveError(null);
    try {
      const saved = props.newType ? await api.createNote(n.type, doc) : await api.updateNote(n.id, doc, base);
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title);
      if (props.newType) {
        navigate(`/notes/${saved.id}`);
        return;
      }
      setNote(saved);
      setConflict(null);
      setEditing(false);
    } catch (e) {
      if (e instanceof ApiError && e.isConflict) {
        try {
          setConflict({ mine: doc, stored: await api.note(n.id) });
        } catch (e2) {
          setSaveError(message(e2));
        }
      } else {
        setSaveError(message(e));
      }
    } finally {
      setSaving(false);
    }
  }

  function onSubmit(ev: SubmitEvent) {
    ev.preventDefault();
    const n = note();
    if (!n) return;
    const t = type();
    const plain: Record<string, Json | undefined> = {};
    let doc = n.note;
    for (const [name, value] of Object.entries(edits())) {
      const f = t?.fields.find((x) => x.name === name);
      if (f?.kind === "dict") doc = applyDict(doc, f, value);
      else plain[name] = value;
    }
    doc = applyEdits(doc, plain);
    for (const { field, blocks } of blockEdits.values()) doc = applyBlocks(doc, field, blocks);
    for (const { field, block } of Object.values(inkEdits())) doc = applyCompound(doc, field, block);
    void save(doc, n.version);
  }

  async function remove() {
    const n = note();
    if (!n || !confirm("Move this note to the trash?")) return;
    setSaving(true);
    setSaveError(null);
    try {
      await api.deleteNote(n.id, n.version);
      navigate(`/types/${n.type}`);
    } catch (e) {
      setSaveError(message(e));
    } finally {
      setSaving(false);
    }
  }

  function keepStored() {
    const c = conflict();
    if (!c) return;
    setNote(c.stored);
    setConflict(null);
    setEditing(false);
  }

  function saveMine() {
    const c = conflict();
    if (c) void save(c.mine, c.stored.version);
  }

  /** A field's value as read-only text: references by their notes' titles. */
  function Display(props: { field: FieldDef; value: Json | undefined }) {
    const ids = () => (Array.isArray(props.value) ? props.value : [props.value]).filter((x) => typeof x === "string");
    return (
      <Show when={props.value !== undefined} fallback={<span class="muted">—</span>}>
        <Show when={props.field.kind === "reference"} fallback={formatValue(props.value)}>
          <For each={ids()}>
            {(id, i) => (
              <>
                {i() > 0 ? ", " : ""}
                <a href={`/notes/${id}`}>{noteTitle(id) ?? id}</a>
              </>
            )}
          </For>
        </Show>
      </Show>
    );
  }

  function Slot(props: { name: string; label: string | null }) {
    const t = type();
    const f = t ? slotField(t, props.name) : undefined;
    return (
      <Show when={f} fallback={<div class="field missing">Unknown field {props.name}</div>}>
        {(field) => {
          const label = props.label ?? field().label;
          const n = () => note() as Note;
          return (
            <div class="field" data-field={field().name} data-testid={`field-${field().name}`}>
              <div class="field-label">
                <label for={`field-input-${field().name}`}>{label}</label>
                <Show when={field().computed}>
                  <span class="badge" title="Only handlers write this field">computed</span>
                </Show>
              </div>
              <div class="field-value">
                <Show when={field().kind !== "canvas"} fallback={
                  <CanvasSurface
                    label={label}
                    block={(editing() && inkEdits()[field().name]?.block) || compoundBlock(n().note, field())}
                    onChange={
                      editing()
                        ? (block) => setInkEdits({ ...inkEdits(), [field().name]: { field: field(), block } })
                        : undefined
                    }
                  />
                }>
                <Show when={field().kind !== "ink"} fallback={
                  <InkSurface
                    label={label}
                    block={(editing() && inkEdits()[field().name]?.block) || compoundBlock(n().note, field())}
                    onChange={
                      editing()
                        ? (block) => setInkEdits({ ...inkEdits(), [field().name]: { field: field(), block } })
                        : undefined
                    }
                  />
                }>
                <Show when={!(editing() && field().kind === "content")} fallback={
                  <ContentEditor
                    label={label}
                    blocks={compoundBlock(n().note, field())?.blocks ?? []}
                    onChange={(blocks) => blockEdits.set(field().name, { field: field(), blocks })}
                  />
                }>
                <Show
                  when={editing() && hasInput(field())}
                  fallback={
                    <Show
                      when={field().compound && field().kind !== "dict"}
                      fallback={<Display field={field()} value={fieldValue(n().note, field())} />}
                    >
                      <Blocks blocks={compoundBlock(n().note, field())?.blocks ?? []} />
                    </Show>
                  }
                >
                  <FieldInput
                    field={field()}
                    value={draftValue(field())}
                    onChange={(v) => setEdits({ ...edits(), [field().name]: v })}
                    label={label}
                    id={`field-input-${field().name}`}
                  />
                </Show>
                </Show>
                </Show>
                </Show>
              </div>
            </div>
          );
        }}
      </Show>
    );
  }

  return (
    <article class="note-view">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={note()} fallback={<Show when={!loadError()}><p class="muted">Loading…</p></Show>}>
        {(n) => (
          <ThisNote.Provider value={n().id}>
            <header class="note-header">
              <div>
                <p class="note-kind">{type()?.label ?? n().type}</p>
                <h1>{props.newType ? `New ${type()?.label ?? n().type}` : formatValue(n().note.fields.title) || "Untitled"}</h1>
              </div>
              <Show when={!conflict()}>
                <Show
                  when={editing()}
                  fallback={
                    <div class="actions">
                      <button
                        type="button"
                        class="pin-button"
                        aria-pressed={isPinned(n().id)}
                        title={isPinned(n().id) ? "Unpin from the sidebar" : "Pin to the sidebar"}
                        onClick={() =>
                          void togglePin(n().id, formatValue(n().note.fields.title) || "Untitled", n().type)
                        }
                      >
                        {isPinned(n().id) ? "Unpin" : "Pin"}
                      </button>
                      <A href={`/notes/${n().id}/history`} class="button">
                        History
                      </A>
                      <button type="button" class="danger" onClick={() => void remove()} disabled={saving()}>
                        Delete
                      </button>
                      <button type="button" onClick={startEditing}>
                        Edit
                      </button>
                    </div>
                  }
                >
                  <div class="actions">
                    <button
                      type="button"
                      onClick={() => (props.newType ? navigate(`/types/${n().type}`) : setEditing(false))}
                      disabled={saving()}
                    >
                      Cancel
                    </button>
                    <button type="submit" form="note-form" class="primary" disabled={saving()}>
                      Save
                    </button>
                  </div>
                </Show>
              </Show>
            </header>
            <Show when={saveError()}>
              <p class="error" role="alert">
                {saveError()}
              </p>
            </Show>
            <Show
              when={conflict()}
              fallback={
                <form id="note-form" onSubmit={onSubmit}>
                  <Show when={tabs().length > 1}>
                    <div class="tabs" role="tablist">
                      <For each={tabs()}>
                        {(t, i) => (
                          <button
                            type="button"
                            role="tab"
                            id={`tab-${i()}`}
                            aria-selected={tab() === i()}
                            aria-controls="tab-panel"
                            onClick={() => setTab(i())}
                          >
                            {t.title}
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>
                  <div
                    id="tab-panel"
                    role={tabs().length > 1 ? "tabpanel" : undefined}
                    aria-labelledby={tabs().length > 1 ? `tab-${tab()}` : undefined}
                    class="tab-panel"
                  >
                    <For each={tabs()[tab()]?.items ?? []}>
                      {(item) =>
                        "slot" in item ? (
                          <Slot name={item.slot} label={item.label} />
                        ) : (
                          <div class="template-block">
                            <BlockView block={item.block} />
                          </div>
                        )
                      }
                    </For>
                  </div>
                </form>
              }
            >
              {(c) => (
                <ConflictView
                  type={type()}
                  mine={c().mine}
                  stored={c().stored.note}
                  busy={saving()}
                  onKeepStored={keepStored}
                  onSaveMine={saveMine}
                />
              )}
            </Show>
          </ThisNote.Provider>
        )}
      </Show>
    </article>
  );
}
