import { useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createSignal, on } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ApiError, type FieldDef, type Json, type Note, type NoteDoc, type Tab, api } from "../api";
import { ConflictView } from "../components/ConflictView";
import { BlockView, Blocks, formatValue } from "../components/Render";
import {
  applyEdits,
  compoundBlock,
  fieldValue,
  isEditable,
  pageTemplate,
  parseInput,
  slotField,
} from "../notes";
import { typeDef } from "../session";

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

/** A note laid out by its type's default template, a tab per template tab (D14). */
export function NoteView() {
  const params = useParams();
  const [note, setNote] = createSignal<Note | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [tab, setTab] = createSignal(0);
  const [editing, setEditing] = createSignal(false);
  const [edits, setEdits] = createStore<Record<string, Json | undefined>>({});
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
    setEdits(reconcile({}));
    setSaveError(null);
    setEditing(true);
  }

  function draftValue(f: FieldDef): Json | undefined {
    const n = note();
    if (f.name in edits) return edits[f.name];
    return n ? n.note.fields[f.name] : undefined;
  }

  async function save(doc: NoteDoc, base: string) {
    const n = note();
    if (!n) return;
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await api.updateNote(n.id, doc, base);
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
    if (n) void save(applyEdits(n.note, { ...edits }), n.version);
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

  function Editor(props: { field: FieldDef; label: string }) {
    const f = props.field;
    const id = `field-input-${f.name}`;
    const value = () => draftValue(f);
    const set = (raw: string) => setEdits(f.name, parseInput(f, raw));
    return (
      <Switch
        fallback={
          <input
            id={id}
            type={{ int: "number", float: "number", date: "date", url: "url" }[f.kind] ?? "text"}
            step={f.kind === "float" ? "any" : undefined}
            value={formatValue(value())}
            required={f.required}
            aria-label={props.label}
            onInput={(e) => set(e.currentTarget.value)}
          />
        }
      >
        <Match when={f.kind === "bool"}>
          <input
            id={id}
            type="checkbox"
            checked={value() === true}
            aria-label={props.label}
            onChange={(e) => setEdits(f.name, e.currentTarget.checked)}
          />
        </Match>
        <Match when={f.kind === "select"}>
          <select id={id} aria-label={props.label} onChange={(e) => set(e.currentTarget.value)}>
            <Show when={!f.required}>
              <option value="" selected={value() === undefined}>
                —
              </option>
            </Show>
            <For each={f.options ?? []}>
              {(o) => (
                <option value={o} selected={value() === o}>
                  {o}
                </option>
              )}
            </For>
          </select>
        </Match>
      </Switch>
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
              </div>
              <div class="field-value">
                <Show
                  when={editing() && isEditable(field())}
                  fallback={
                    <Show
                      when={field().compound}
                      fallback={
                        <Show
                          when={fieldValue(n().note, field()) !== undefined}
                          fallback={<span class="muted">—</span>}
                        >
                          {formatValue(fieldValue(n().note, field()))}
                        </Show>
                      }
                    >
                      <Blocks blocks={compoundBlock(n().note, field())?.blocks ?? []} />
                    </Show>
                  }
                >
                  <Editor field={field()} label={label} />
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
          <>
            <header class="note-header">
              <div>
                <p class="note-kind">{type()?.label ?? n().type}</p>
                <h1>{formatValue(n().note.fields.title) || "Untitled"}</h1>
              </div>
              <Show when={!conflict()}>
                <Show
                  when={editing()}
                  fallback={
                    <button type="button" onClick={startEditing}>
                      Edit
                    </button>
                  }
                >
                  <div class="actions">
                    <button type="button" onClick={() => setEditing(false)} disabled={saving()}>
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
          </>
        )}
      </Show>
    </article>
  );
}
