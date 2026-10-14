import { A, useNavigate, useParams, useSearchParams } from "@solidjs/router";
import { For, type JSX, Show, batch, createEffect, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { ApiError, type Block, type FieldDef, type Json, type Note, type NoteDoc, type Tab, type TypeDef, api } from "../api";
import { type Autosave, type SaveState, createAutosave } from "../autosave";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { ConflictView } from "../components/ConflictView";
import { type ControlBinding, ControlBindingContext } from "../controls/binding";
import { FieldInput, hasInput } from "../components/FieldInput";
import { FieldRefs, type FieldRefValues } from "../components/fieldRefs";
import { refText } from "../components/refText";
import { BlockView, Blocks, ColumnsView, formatValue } from "../components/Render";
import { slotOfBlock } from "../blocks/columns";
import { ContentEditor } from "../editor/ContentEditor";
import { knowTitle, noteTitle } from "../editor/titles";
import { InkSurface } from "../ink/InkSurface";
import {
  applyBlocks,
  applyCompound,
  applyDict,
  applyEdits,
  compoundBlock,
  fieldValue,
  missingRequired,
  pageTemplate,
  slotField,
  TITLE_FIELD,
  unplacedFields,
} from "../notes";
import { changes } from "../changes";
import { isPinned, togglePin } from "../pins";
import { typeDef } from "../session";
import { focusPanel, offerProperties, rightIsDrawer, sidebarShown, sidebars, toggleSidebar } from "../sidebars";
import { activePane } from "../workspace/model";
import { usePane } from "../workspace/pane";
import { workspace } from "../workspace/store";
import { ThisNote } from "../views/thisNote";
import { TrashIcon } from "../components/Icon";

/** How long after the last change a note saves itself. */
export const AUTOSAVE_MS = 1500;

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

/** The page for a new note of the type named in `?type=`: its default template, empty (D14). */
export function NewNote() {
  const [search] = useSearchParams();
  const type = () => (typeof search.type === "string" ? search.type : "");
  return (
    <Show when={typeDef(type())} keyed fallback={<p class="error">There is no type {type()}.</p>}>
      {(t) => <NoteView newType={t.name} />}
    </Show>
  );
}

/** A draft of a note of `type`, holding each plain field's default. */
function draftNote(type: TypeDef): Note {
  const fields: Record<string, Json> = {};
  for (const f of type.fields) if (!f.compound && !f.computed && f.default !== undefined) fields[f.name] = f.default;
  return { id: "", type: type.name, version: "", wcl: "", note: { fields, blocks: [] } };
}

/** A pin, filled once the note is pinned. */
function PinIcon(props: { filled: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill={props.filled ? "currentColor" : "none"} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 17v5" />
      <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
    </svg>
  );
}

/** A clock turning back, for the note's history. */
function HistoryIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 7v5l4 2" />
    </svg>
  );
}

/** The sliders icon of the properties button. */
function PropertiesIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
      <line x1="4" y1="6" x2="20" y2="6" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="18" x2="20" y2="18" />
      <circle cx="9" cy="6" r="2" fill="var(--bg-raised, Canvas)" />
      <circle cx="15" cy="12" r="2" fill="var(--bg-raised, Canvas)" />
      <circle cx="7" cy="18" r="2" fill="var(--bg-raised, Canvas)" />
    </svg>
  );
}

/**
 * A note laid out by its type's default template, a tab per template tab (D14), and edited in
 * place: every field the template places is edited where it shows, the rest in the properties
 * sidebar, and changes save themselves (D12 versioning; a stale save shows the conflict view).
 * With `newType`, an unsaved note of that type, which Create creates.
 */
/**
 * Fit the title box to its text where the browser cannot (no `field-sizing`): as tall as its
 * wrapped lines. Where it can, CSS does this and this does nothing.
 */
function fitTitle(el: HTMLTextAreaElement) {
  if (typeof CSS !== "undefined" && CSS.supports?.("field-sizing", "content")) return;
  const fit = () => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  };
  queueMicrotask(fit);
  if (!el.dataset.fitted && typeof ResizeObserver !== "undefined") {
    el.dataset.fitted = "1";
    new ResizeObserver(fit).observe(el);
  }
}

export function NoteView(props: { newType?: string; id?: string }) {
  const params = useParams();
  const navigate = useNavigate();
  const pane = usePane();
  // Element ids are per pane, so the same note open twice keeps its labels on its own fields.
  const uid = pane ? `${pane.id}-` : "";
  // The note as last stored, and the note as edited.
  const [note, setNote] = createSignal<Note | null>(null);
  const [doc, setDoc] = createSignal<NoteDoc>({ fields: {}, blocks: [] });
  // Bumped when the edited note is replaced from outside, so editors that keep their own state
  // (the block editor) start again from it.
  const [generation, setGeneration] = createSignal(0);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [tab, setTab] = createSignal(0);
  const [saveState, setSaveState] = createSignal<SaveState>("idle");
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [conflict, setConflict] = createSignal<Conflict | null>(null);
  let autosave: Autosave | null = null;
  let loadedId = "";

  /** Whether the right sidebar shows this note's properties now. */
  const propertiesShown = () =>
    sidebarShown("right", rightIsDrawer()) && sidebars().right.panel === "properties" && (!pane || activePane(workspace()) === pane.id);

  /** The properties button: open the right sidebar on this note's properties, or close it. */
  function toggleProperties() {
    if (propertiesShown()) toggleSidebar("right", false);
    else focusPanel("properties");
  }

  /** The autosave for the note `id`: each save sends what is edited now over the last version. */
  function autosaveFor(id: string): Autosave {
    const saver: Autosave = createAutosave({
      delay: AUTOSAVE_MS,
      onState: (s) => {
        if (loadedId === id) setSaveState(s);
      },
      save: async () => {
        const n = note();
        if (!n || n.id !== id || conflict()) return;
        const mine = doc();
        setSaveError(null);
        try {
          const saved = await api.updateNote(id, mine, n.version);
          const title = saved.note.fields.title;
          if (typeof title === "string") knowTitle(saved.id, title, saved.type);
          if (loadedId !== id) return;
          setNote(saved);
          // Take the server's form of the note (computed fields, defaults) unless it was edited since.
          if (!saver.dirty()) setDoc(saved.note);
        } catch (e) {
          if (loadedId !== id) throw e;
          if (e instanceof ApiError && e.isConflict) {
            saver.cancel();
            try {
              // Everything edited so far is yours, including changes made while this save ran.
              const stored = await api.note(id);
              setConflict({ mine: doc(), stored });
            } catch (e2) {
              setSaveError(message(e2));
            }
            return;
          }
          setSaveError(message(e));
          throw e;
        }
      },
    });
    return saver;
  }

  createEffect(
    on(
      () => props.id ?? params.id,
      async (id) => {
        // Whatever the last note had unsaved goes now, over that note.
        void autosave?.flush();
        autosave = null;
        loadedId = props.newType ? "" : (id ?? "");
        setNote(null);
        setLoadError(null);
        setSaveError(null);
        setSaveState("idle");
        setTab(0);
        setConflict(null);
        if (props.newType) {
          const t = typeDef(props.newType);
          if (!t) return;
          const draft = draftNote(t);
          setDoc(draft.note);
          setNote(draft);
          return;
        }
        try {
          const loaded = await api.note(id ?? "");
          if (loadedId !== (id ?? "")) return;
          const loadedTitle = loaded.note.fields.title;
          if (typeof loadedTitle === "string") knowTitle(loaded.id, loadedTitle, loaded.type);
          batch(() => {
            setDoc(loaded.note);
            setNote(loaded);
            setGeneration(generation() + 1);
          });
          autosave = autosaveFor(loaded.id);
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? "This note does not exist." : message(e));
        }
      },
    ),
  );

  // A write elsewhere in this client (another pane on the same note, a card, a script field) may
  // have changed this note. Unless it has edits of its own, which will meet the conflict view on
  // their next save, it takes the stored note.
  let recheck: ReturnType<typeof setTimeout> | undefined;
  const untouched = () => !props.newType && !conflict() && !autosave?.dirty() && saveState() !== "saving" && saveState() !== "dirty";
  createEffect(
    on(
      changes,
      () => {
        clearTimeout(recheck);
        recheck = setTimeout(async () => {
          const n = note();
          if (!n || !n.id || !untouched()) return;
          try {
            const stored = await api.note(n.id);
            if (note()?.id !== stored.id || note()?.version === stored.version || !untouched()) return;
            batch(() => {
              setNote(stored);
              setDoc(stored.note);
              setGeneration(generation() + 1);
            });
          } catch {
            // Deleted or unreachable: the next save says so.
          }
        }, 250);
      },
      { defer: true },
    ),
  );
  onCleanup(() => clearTimeout(recheck));

  // The pane header and tab name the note this page shows (not a daily note's, which names its day).
  // Read once: a page given its note by a parent (a daily note) is named by that parent.
  const routed = untrack(() => props.id === undefined);
  createEffect(() => {
    if (!pane || !routed) return;
    const n = note();
    if (!n) return;
    const t = type();
    const title = typeof doc().fields.title === "string" ? (doc().fields.title as string) : "";
    pane.setTitle({ title: props.newType ? `New ${t?.label.toLowerCase() ?? n.type}` : title || "Untitled", icon: t?.icon ?? null });
  });
  onCleanup(() => {
    if (pane && routed) pane.setTitle(null);
  });

  const flush = () => void autosave?.flush();
  /** Leaving a field saves what it changed; moving within the block editor does not. */
  const flushOnLeave = (e: FocusEvent) => {
    const to = e.relatedTarget;
    if (!(to instanceof Node && e.target instanceof Element && e.target.closest(".content-editor")?.contains(to))) flush();
  };

  // The right sidebar's Properties panel edits the fields the template places nowhere, through
  // this page's own note and autosave, so the two never disagree.
  const renderProperties = () => (
    <div class="properties-fields" onFocusOut={flushOnLeave}>
      <Show when={note()} fallback={<p class="muted">Loading…</p>}>
        {(n) => (
          <ThisNote.Provider value={n().id || undefined}>
            <Show when={!conflict()} fallback={<p class="muted">Resolve the conflict in the pane first.</p>}>
              <Show when={generation() + 1} keyed>
                {(_) => (
                  <For each={type() ? unplacedFields(type()!, doc()) : []} fallback={<p class="muted">Every property is on the page.</p>}>
                    {(f) => <Property field={f} label={f.label} ns="properties-" />}
                  </For>
                )}
              </Show>
            </Show>
          </ThisNote.Provider>
        )}
      </Show>
    </div>
  );
  if (pane) onCleanup(offerProperties(pane.id, { noteId: () => note()?.id ?? "", render: renderProperties }));

  const onHidden = () => {
    if (document.visibilityState === "hidden") flush();
  };
  onMount(() => {
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("beforeunload", flush);
  });
  onCleanup(() => {
    document.removeEventListener("visibilitychange", onHidden);
    window.removeEventListener("beforeunload", flush);
    flush();
  });

  const type = () => {
    const n = note();
    return n ? typeDef(n.type) : undefined;
  };
  const tabs = (): Tab[] => {
    const t = type();
    return (t && pageTemplate(t)?.tabs) ?? [];
  };

  /** Apply an edit to the note, and save it after a pause (a draft waits for Create). */
  function edit(fn: (d: NoteDoc) => NoteDoc) {
    setDoc(fn(doc()));
    autosave?.schedule();
  }

  function setField(f: FieldDef, v: Json | undefined) {
    edit((d) => (f.kind === "dict" ? applyDict(d, f, v) : applyEdits(d, { [f.name]: v })));
  }

  const missing = () => {
    const t = type();
    return t ? missingRequired(t, doc()) : [];
  };

  async function create() {
    const n = note();
    if (!n || missing().length) return;
    setBusy(true);
    setSaveError(null);
    try {
      const saved = await api.createNote(n.type, doc());
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title, saved.type);
      navigate(`/notes/${saved.id}`);
    } catch (e) {
      setSaveError(message(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    const n = note();
    if (!n || !confirm("Move this note to the trash?")) return;
    setBusy(true);
    setSaveError(null);
    try {
      await autosave?.flush();
      autosave?.cancel();
      await api.deleteNote(n.id, note()?.version ?? n.version);
      navigate(`/types/${n.type}`);
    } catch (e) {
      setSaveError(message(e));
    } finally {
      setBusy(false);
    }
  }

  function keepStored() {
    const c = conflict();
    if (!c) return;
    setNote(c.stored);
    setDoc(c.stored.note);
    setGeneration(generation() + 1);
    setConflict(null);
    setSaveState("idle");
  }

  async function saveMine() {
    const c = conflict();
    if (!c) return;
    setBusy(true);
    setSaveError(null);
    try {
      const saved = await api.updateNote(c.stored.id, c.mine, c.stored.version);
      setNote(saved);
      setDoc(saved.note);
      setGeneration(generation() + 1);
      setConflict(null);
      setSaveState("saved");
    } catch (e) {
      if (e instanceof ApiError && e.isConflict) {
        setConflict({ mine: c.mine, stored: await api.note(c.stored.id) });
      } else {
        setSaveError(message(e));
      }
    } finally {
      setBusy(false);
    }
  }

  /** `{{name}}` in the template's fixed blocks shows the note's values as edited. */
  const refs: FieldRefValues = { text: (name) => refText(type(), doc(), name) };

  const status = (): string => {
    if (props.newType) return missing().length ? `Required: ${missing().join(", ")}` : "";
    switch (saveState()) {
      case "dirty":
        return "Unsaved changes";
      case "saving":
        return "Saving…";
      case "saved":
        return "Saved";
      case "error":
        return saveError() ?? "Could not save";
      default:
        return "";
    }
  };

  // Controls anywhere on the page (the template, the note's content, a canvas card) edit this
  // note's fields through the same edit path as the field editors.
  const controls: ControlBinding = {
    type: () => type(),
    value: (f) => fieldValue(doc(), f),
    set: (f, v) => setField(f, v),
    noteId: () => note()?.id || undefined,
  };

  return (
    <ControlBindingContext.Provider value={controls}>
    <article
      class="note-view"
      data-save-state={saveState()}
      onFocusOut={flushOnLeave}
    >
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={note()} fallback={<Show when={!loadError()}><p class="muted">Loading…</p></Show>}>
        {(n) => (
          <ThisNote.Provider value={n().id || undefined}>
            <header class="note-header">
              <div class="note-heading">
                <p class="note-kind">{props.newType ? `New ${type()?.label ?? n().type}` : (type()?.label ?? n().type)}</p>
                <div class="note-title-row">
                  <h1 class="note-title">
                    {/* A textarea, so a long title wraps in a narrow pane rather than scrolling out of sight. */}
                    <textarea
                      id={`${uid}note-title`}
                      class="note-title-input"
                      aria-label="Title"
                      placeholder="Untitled"
                      required
                      rows={1}
                      autocomplete="off"
                      spellcheck={false}
                      value={typeof doc().fields.title === "string" ? (doc().fields.title as string) : ""}
                      ref={(el) => fitTitle(el)}
                      onInput={(e) => {
                        const el = e.currentTarget;
                        // A title is one line: a pasted line break becomes a space.
                        if (/[\r\n]/.test(el.value)) el.value = el.value.replace(/\s*[\r\n]+\s*/g, " ");
                        setField(TITLE_FIELD, el.value === "" ? undefined : el.value);
                        fitTitle(el);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          flush();
                        }
                      }}
                    />
                  </h1>
                  <button
                    type="button"
                    class="icon-button properties-toggle"
                    aria-label="Properties"
                    title="Properties"
                    aria-expanded={propertiesShown()}
                    aria-controls="properties-panel"
                    onClick={toggleProperties}
                  >
                    <PropertiesIcon />
                  </button>
                  <Show when={!props.newType}>
                    <button
                      type="button"
                      class="icon-button pin-button"
                      aria-label={isPinned(n().id) ? "Unpin" : "Pin"}
                      aria-pressed={isPinned(n().id)}
                      title={isPinned(n().id) ? "Unpin from the sidebar" : "Pin to the sidebar"}
                      onClick={() => void togglePin(n().id, formatValue(doc().fields.title) || "Untitled", n().type)}
                    >
                      <PinIcon filled={isPinned(n().id)} />
                    </button>
                    <A href={`/notes/${n().id}/history`} class="icon-button" aria-label="History" title="History">
                      <HistoryIcon />
                    </A>
                    <button
                      type="button"
                      class="icon-button danger"
                      aria-label="Delete"
                      title="Delete note"
                      onClick={() => void remove()}
                      disabled={busy()}
                    >
                      <TrashIcon />
                    </button>
                  </Show>
                </div>
              </div>
              <div class="note-actions">
                <span
                  class="save-status"
                  classList={{ error: saveState() === "error" }}
                  role="status"
                  data-testid="save-status"
                >
                  {status()}
                </span>
                <Show when={props.newType}>
                  <div class="actions">
                      <button type="button" onClick={() => navigate(`/types/${n().type}`)} disabled={busy()}>
                        Cancel
                      </button>
                      <button
                        type="button"
                        class="primary"
                        disabled={busy() || missing().length > 0}
                        title={missing().length ? `Fill in ${missing().join(", ")} first` : undefined}
                        onClick={() => void create()}
                      >
                        Create
                      </button>
                    </div>
                </Show>
              </div>
            </header>
            <Show when={saveError() && (props.newType || conflict())}>
              <p class="error" role="alert">
                {saveError()}
              </p>
            </Show>
            <Show
              when={conflict()}
              fallback={
                <div class="note-layout">
                  <div class="note-main">
                    <Show when={tabs().length > 1}>
                      <div class="tabs" role="tablist">
                        <For each={tabs()}>
                          {(t, i) => (
                            <button
                              type="button"
                              role="tab"
                              id={`${uid}tab-${i()}`}
                              aria-selected={tab() === i()}
                              aria-controls={`${uid}tab-panel`}
                              onClick={() => {
                                flush();
                                setTab(i());
                              }}
                            >
                              {t.title}
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                    {/* Keyed on the generation: a note replaced from outside remounts its editors. */}
                    <Show when={generation() + 1} keyed>
                      {(_) => (
                        <div
                          id={`${uid}tab-panel`}
                          role={tabs().length > 1 ? "tabpanel" : undefined}
                          aria-labelledby={tabs().length > 1 ? `${uid}tab-${tab()}` : undefined}
                          class="tab-panel"
                        >
                          <For each={tabs()[tab()]?.items ?? []}>
                            {(item) =>
                              "slot" in item ? (
                                // The heading is the title's editor; a title slot would repeat it.
                                item.slot === "title" ? null : <Slot name={item.slot} label={item.label} />
                              ) : (
                                <FieldRefs.Provider value={refs}>
                                  <Show
                                    when={item.block.kind === "columns"}
                                    fallback={
                                      <div class="template-block" data-testid="template-block">
                                        <BlockView block={item.block} />
                                      </div>
                                    }
                                  >
                                    {/* Fields side by side: the slots in a layout's columns are edited in place too. */}
                                    <div class="template-columns" data-testid="template-columns">
                                      <ColumnsView block={item.block} child={templateChild} />
                                    </div>
                                  </Show>
                                </FieldRefs.Provider>
                              )
                            }
                          </For>
                        </div>
                      )}
                    </Show>
                  </div>
                </div>
              }
            >
              {(c) => (
                <ConflictView
                  type={type()}
                  mine={c().mine}
                  stored={c().stored.note}
                  busy={busy()}
                  onKeepStored={keepStored}
                  onSaveMine={() => void saveMine()}
                />
              )}
            </Show>
          </ThisNote.Provider>
        )}
      </Show>
    </article>
    </ControlBindingContext.Provider>
  );

  /** A field's value as read-only text: references by their notes' titles. */
  function Display(p: { field: FieldDef; value: Json | undefined }) {
    const ids = () => (Array.isArray(p.value) ? p.value : [p.value]).filter((x) => typeof x === "string");
    return (
      <Show when={p.value !== undefined} fallback={<span class="muted">—</span>}>
        <Show when={p.field.kind === "reference"} fallback={formatValue(p.value)}>
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

  /** The editor for one field, in place: chosen by the field's kind. */
  function FieldEditor(p: { field: FieldDef; label: string; ns: string }): JSX.Element {
    const f = p.field;
    const id = `${p.ns}field-input-${f.name}`;
    if (f.computed) {
      return (
        <Show when={f.compound && f.kind !== "dict"} fallback={<Display field={f} value={fieldValue(doc(), f)} />}>
          <Blocks blocks={compoundBlock(doc(), f)?.blocks ?? []} />
        </Show>
      );
    }
    switch (f.kind) {
      case "content":
        return (
          <ContentEditor
            label={p.label}
            blocks={compoundBlock(doc(), f)?.blocks ?? []}
            onChange={(blocks) => edit((d) => applyBlocks(d, f, blocks))}
          />
        );
      case "ink":
        return (
          <InkSurface
            label={p.label}
            block={compoundBlock(doc(), f)}
            onChange={(block) => edit((d) => applyCompound(d, f, block))}
          />
        );
      case "canvas":
        return (
          <CanvasSurface
            label={p.label}
            block={compoundBlock(doc(), f)}
            onChange={(block) => edit((d) => applyCompound(d, f, block))}
          />
        );
    }
    if (hasInput(f)) {
      return <FieldInput
          field={f}
          value={fieldValue(doc(), f)}
          onChange={(v) => setField(f, v)}
          label={p.label}
          id={id}
          placeholder="Empty"
        />;
    }
    return (
      <Show when={f.compound && f.kind !== "dict"} fallback={<Display field={f} value={fieldValue(doc(), f)} />}>
        <Blocks blocks={compoundBlock(doc(), f)?.blocks ?? []} />
      </Show>
    );
  }

  /** A property block: a field's label and its editor, in place. */
  function Property(p: { field: FieldDef; label: string; ns?: string }) {
    const ns = p.ns ?? uid;
    const f = p.field;
    const wide = f.kind === "content" || f.kind === "ink" || f.kind === "canvas" || f.kind === "location";
    return (
      <div
        class="field property"
        classList={{ wide, [`kind-${f.kind}`]: true }}
        data-field={f.name}
        data-testid={`field-${f.name}`}
      >
        <div class="field-label">
          <Show when={hasInput(f)} fallback={<span>{p.label}</span>}>
            <label for={`${ns}field-input-${f.name}`}>{p.label}</label>
          </Show>
          <Show when={f.computed}>
            <span class="badge" title="Only handlers write this field">
              computed
            </span>
          </Show>
        </div>
        <div class="field-value">
          <FieldEditor field={f} label={p.label} ns={ns} />
        </div>
      </div>
    );
  }

  /** A block in a column of the template: a slot's property editor, or a fixed block. */
  function templateChild(b: Block): JSX.Element {
    const slot = slotOfBlock(b);
    if (slot === null) {
      return (
        <div class="template-block" data-testid="template-block">
          <BlockView block={b} />
        </div>
      );
    }
    if (slot === "title") return null;
    const label = b.fields?.label;
    return <Slot name={slot} label={typeof label === "string" ? label : null} />;
  }

  function Slot(p: { name: string; label: string | null }) {
    const t = type();
    const f = t ? slotField(t, p.name) : undefined;
    return (
      <Show when={f} fallback={<div class="field missing">Unknown field {p.name}</div>}>
        {(field) => <Property field={field()} label={p.label ?? field().label} />}
      </Show>
    );
  }
}
