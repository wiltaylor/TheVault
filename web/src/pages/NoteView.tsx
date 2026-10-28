import { useNavigate, useParams, useSearchParams } from "@solidjs/router";
import { For, type JSX, Show, batch, createEffect, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { ApiError, type Block, type FieldDef, type Json, type Note, type NoteDoc, type Tab, type TypeDef, api, conflictOf, isArchived } from "../api";
import { clearActiveNote, setActiveNote } from "../archive";
import { type Autosave, type SaveState, createAutosave } from "../autosave";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { AssetInput } from "../components/AssetField";
import { ConflictView } from "../components/ConflictView";
import { type ControlBinding, ControlBindingContext } from "../controls/binding";
import { FieldInput, type RelatedFields, hasInput } from "../components/FieldInput";
import { FieldRefs, type FieldRefValues } from "../components/fieldRefs";
import { refText } from "../components/refText";
import { BlockView, Blocks, ColumnsView, FieldValue, formatValue } from "../components/Render";
import { slotOfBlock } from "../blocks/columns";
import { ContentEditor } from "../editor/ContentEditor";
import { knowArchived, knowTitle, noteTitle } from "../editor/titles";
import { InkSurface } from "../ink/InkSurface";
import {
  applyBlocks,
  applyCompound,
  applyDict,
  applyEdits,
  compoundBlock,
  fieldValue,
  missingRequired,
  mergeDocs,
  pageTemplate,
  propertyFields,
  sameBlocks,
  sameValue,
  slotField,
  TITLE_FIELD,
  whenNote,
} from "../notes";
import { visibleTabs, whenHolds } from "../when";
import { changes, onNoteChangedElsewhere } from "../changes";
import { isPinned, togglePin } from "../pins";
import { typeDef } from "../session";
import { focusPanel, offerNote, panelShown, rightIsDrawer, toggleSidebar } from "../sidebars";
import { authorText, when } from "../admin/format";
import { type Preview, previewOf, setPreview } from "../history/preview";
import { restoreVersion } from "../history/restore";
import { activePane } from "../workspace/model";
import { usePane } from "../workspace/pane";
import { workspace } from "../workspace/store";
import { ThisNote } from "../views/thisNote";
import { setFindText } from "../blocks/toggle";
import { offerPlaceholders, replacePlaceholderInDoc, resolvePlaceholder } from "../placeholders";
import { PlaceholdersInert } from "../components/PlaceholderLink";
import { ArchiveIcon, TrashIcon, TypeIcon, UnarchiveIcon } from "../components/Icon";
import { NoteOutboxActions } from "../outbox/NoteActions";
import { OutboxChips } from "../outbox/Chips";
import { OUTBOX } from "../outbox/model";
import { periodOfType } from "../planner/period";
import { SERIES_TYPE } from "../series/rrule";

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
  const param = (name: string) => {
    const v = search[name];
    return typeof v === "string" ? v : "";
  };
  const type = () => param("type");
  // `title` fills in the title; `from` names the note whose placeholder link for this type and
  // title opened the form, rewritten as a mention once the note is created.
  const prefill = () => (param("title") ? { title: param("title"), from: param("from") || undefined } : undefined);
  return (
    <Show when={typeDef(type())} keyed fallback={<p class="error">There is no type {type()}.</p>}>
      {(t) => <NoteView newType={t.name} prefill={prefill()} />}
    </Show>
  );
}

/** A draft of a note of `type`, holding each plain field's default. */
function draftNote(type: TypeDef): Note {
  const fields: Record<string, Json> = {};
  for (const f of type.fields) if (!f.compound && !f.computed && f.default !== undefined) fields[f.name] = f.default;
  // A new event series repeats in this browser's time zone unless the user says otherwise
  // (event-series E1).
  if (type.name === SERIES_TYPE && fields.timezone === undefined && type.fields.some((f) => f.name === "timezone")) {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (zone) fields.timezone = zone;
  }
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
 * place: every field the template places is edited where it shows, and the properties sidebar
 * edits every field as well, and changes save themselves (D12 versioning; a stale save shows the
 * conflict view). With `newType`, an unsaved note of that type, which Create creates.
 */
export function NoteView(props: { newType?: string; id?: string; prefill?: { title: string; from?: string } }) {
  const params = useParams();
  // `?find=`: the text a search opened the note with. A toggle holding it opens (blocks/toggle.ts).
  const [query] = useSearchParams();
  createEffect(() => {
    const find = query.find;
    setFindText(props.id ?? params.id ?? "", typeof find === "string" ? find : null);
  });
  onCleanup(() => setFindText("", null));
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

  /** Whether the right sidebar shows this note's properties, or its history, now. */
  const isActive = () => !pane || activePane(workspace()) === pane.id;
  const propertiesShown = () => panelShown("properties") && isActive();
  const historyShown = () => panelShown("history") && isActive();

  /** The properties button: open the right sidebar on this note's properties, or close it. */
  function toggleProperties() {
    if (propertiesShown()) toggleSidebar("right", false);
    else focusPanel("properties");
  }

  /** The history button: open the right sidebar on this note's versions, or close it. */
  function toggleHistory() {
    if (historyShown()) toggleSidebar("right", false);
    else focusPanel("history");
  }

  /** The version of this note the History panel shows in this pane, read-only, if any. */
  const preview = (): Preview | undefined => {
    const p = pane ? previewOf(pane.id) : undefined;
    const n = note();
    return p && n && p.noteId === n.id ? p : undefined;
  };
  /** The note as the page shows it: the previewed version, or the note as edited. */
  const shown = (): NoteDoc => preview()?.note.note ?? doc();

  // The title is editable text, not an input, so its text is set here: from the note as shown,
  // except while someone is typing in it, so the caret never jumps.
  let titleEl: HTMLSpanElement | undefined;
  createEffect(() => {
    const t = shown().fields.title;
    const text = typeof t === "string" ? t : "";
    if (titleEl && (titleEl.textContent ?? "") !== text && document.activeElement !== titleEl) titleEl.textContent = text;
  });
  /**
   * Show a title changed elsewhere even while the title has focus, caret at the end, so the next
   * keystroke edits that title rather than writing the one on screen back over it.
   */
  function showOutsideTitle() {
    const t = doc().fields.title;
    const text = typeof t === "string" ? t : "";
    if (!titleEl || preview() || (titleEl.textContent ?? "") === text) return;
    titleEl.textContent = text;
    if (document.activeElement !== titleEl) return;
    const range = document.createRange();
    range.selectNodeContents(titleEl);
    range.collapse(false);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
  }
  const endPreview = () => {
    if (pane) setPreview(pane.id, null);
  };

  /** The autosave for the note `id`: each save sends what is edited now over the last version. */
  function autosaveFor(id: string): Autosave {
    const saver: Autosave = createAutosave({
      delay: AUTOSAVE_MS,
      onState: (s) => {
        if (loadedId === id) setSaveState(s);
      },
      save: async () => {
        const n = note();
        if (!n || n.id !== id) return;
        // Never "saved" while the conflict view holds the edits: nothing is sent until it is resolved.
        if (conflict()) throw new Error("Resolve the conflict first.");
        // What the page edited, and the stored note it edited from.
        const edited = doc();
        let base = n;
        let sending = edited;
        // Whether a merge took a change made elsewhere into the note's compound blocks, or its title.
        let tookBlocks = false;
        let tookTitle = false;
        setSaveError(null);
        try {
          let saved: Note | null = null;
          for (let attempt = 0; !saved; attempt++) {
            try {
              saved = await api.updateNote(id, sending, base.version);
            } catch (e) {
              if (!(e instanceof ApiError && e.isConflict) || attempt >= 3) throw e;
              // Written elsewhere since (a sync round taking the server's result back, another
              // device, a script): what this page changed goes over it, unless both changed the
              // same field, which is the conflict view's.
              const stored = await api.note(id);
              if (loadedId !== id) throw e;
              const merged = mergeDocs(base.note, sending, stored.note);
              if (!merged) throw e;
              if (!sameBlocks(merged, sending)) tookBlocks = true;
              if (!sameValue(merged.fields.title, sending.fields.title)) tookTitle = true;
              base = stored;
              sending = merged;
            }
          }
          const title = saved.note.fields.title;
          if (typeof title === "string") knowTitle(saved.id, title, saved.type);
          if (loadedId !== id) return;
          // Take the server's form of the note (computed fields, defaults, the changes merged in),
          // keeping whatever was edited while the save ran, which the next save sends.
          const now = doc();
          const next = now === edited ? saved.note : (mergeDocs(edited, now, saved.note, true) ?? now);
          batch(() => {
            setNote(saved);
            setDoc(next);
            // An editor with state of its own starts again from a block changed elsewhere.
            if (tookBlocks) setGeneration(generation() + 1);
          });
          if (tookTitle) showOutsideTitle();
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
        endPreview();
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
          const title = untrack(() => props.prefill?.title);
          if (title) draft.note.fields.title = title;
          setDoc(draft.note);
          setNote(draft);
          return;
        }
        try {
          const loaded = await api.note(id ?? "");
          if (loadedId !== (id ?? "")) return;
          const loadedTitle = loaded.note.fields.title;
          if (typeof loadedTitle === "string") knowTitle(loaded.id, loadedTitle, loaded.type);
          knowArchived(loaded.id, isArchived(loaded.note.fields));
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

  // A write elsewhere in this client (another pane on the same note, a card, a script field), or
  // on another device as the live stream reports it, may have changed this note. Unless it has
  // edits of its own, which its next save merges with the stored note (or, when both changed the
  // same field, shows in the conflict view), it takes the stored note.
  let recheck: ReturnType<typeof setTimeout> | undefined;
  const untouched = () => !props.newType && !conflict() && !autosave?.dirty() && saveState() !== "saving" && saveState() !== "dirty";
  const recheckSoon = () => {
    clearTimeout(recheck);
    recheck = setTimeout(async () => {
      const n = note();
      if (!n || !n.id || !untouched()) return;
      const before = doc();
      try {
        const stored = await api.note(n.id);
        // An edit made while the note was on its way wins: its save merges it with the stored note.
        if (note()?.id !== stored.id || note()?.version === stored.version || doc() !== before || !untouched()) return;
        batch(() => {
          setNote(stored);
          setDoc(stored.note);
          // The block editors keep their own state: they start again only when a block changed,
          // so a write that changed nothing they show (a sync round taking the same note back
          // with a new version) leaves the caret where it is.
          if (!sameBlocks(before, stored.note)) setGeneration(generation() + 1);
        });
        showOutsideTitle();
      } catch {
        // Deleted or unreachable: the next save says so.
      }
    }, 250);
  };
  createEffect(on(changes, recheckSoon, { defer: true }));
  onCleanup(
    onNoteChangedElsewhere((change) => {
      const n = note();
      if (!n || (change && (change.id !== n.id || change.version === n.version))) return;
      recheckSoon();
    }),
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

  // The right sidebar's Properties panel edits every field but the hidden ones and the bodies the
  // page already shows (notes.ts, propertyFields), through this page's own note and autosave, so a
  // field edited in one place shows the change in the other.
  const renderProperties = () => (
    <div class="properties-fields" onFocusOut={flushOnLeave}>
      <Show when={note()} fallback={<p class="muted">Loading…</p>}>
        {(n) => (
          <ThisNote.Provider value={n().id || undefined}>
            <Show when={!conflict()} fallback={<p class="muted">Resolve the conflict in the pane first.</p>}>
              <Show when={layoutKey()} keyed>
                {(_) => (
                  <fieldset class="preview-lock" disabled={!!preview()}>
                    <Show when={preview()}>
                      <p class="muted preview-note">Showing the version from {when(preview()!.at)}, read-only.</p>
                    </Show>
                    <For each={type() ? propertyFields(type()!, shown()) : []} fallback={<p class="muted">This type has no properties to show.</p>}>
                      {(f) => <Property field={f} label={f.label} ns="properties-" />}
                    </For>
                  </fieldset>
                )}
              </Show>
            </Show>
          </ThisNote.Provider>
        )}
      </Show>
    </div>
  );
  /** A restore wrote `saved`: it is the live note now. */
  function takeRestored(saved: Note) {
    if (saved.id !== note()?.id) return;
    autosave?.cancel();
    batch(() => {
      setNote(saved);
      setDoc(saved.note);
      setGeneration(generation() + 1);
      setConflict(null);
      setSaveState("idle");
      endPreview();
    });
    showOutsideTitle();
  }

  // A placeholder link clicked outside the editor (a card, the new-note form) is rewritten here,
  // with this page's own edits, rather than over them.
  createEffect(() => {
    const id = note()?.id;
    if (!id || props.newType) return;
    onCleanup(
      offerPlaceholders(id, (noteType, title, newId) => {
        if (preview() || conflict()) return false;
        const next = replacePlaceholderInDoc(doc(), noteType, title, newId);
        if (!next) return false;
        batch(() => {
          setDoc(next);
          setGeneration(generation() + 1);
        });
        autosave?.schedule();
        return true;
      }),
    );
  });

  if (pane) {
    onCleanup(
      offerNote(pane.id, {
        noteId: () => note()?.id ?? "",
        version: () => note()?.version ?? "",
        render: renderProperties,
        flush: async () => {
          await autosave?.flush();
          return !conflict() && !autosave?.dirty();
        },
        restored: takeRestored,
        day: () => {
          const n = note();
          const date = periodOfType(n?.type) ? n!.note.fields.date : undefined;
          return typeof date === "string" ? date.slice(0, 10) : "";
        },
      }),
    );
    // Leaving the note (another page in the pane, the pane closing) ends its preview.
    onCleanup(endPreview);
  }

  /** Restore the previewed version, after asking. */
  async function restorePreview() {
    const p = preview();
    const n = note();
    if (!p || !n || !pane) return;
    setSaveError(null);
    const err = await restoreVersion(pane.id, p);
    if (err) setSaveError(err);
  }

  // Escape anywhere returns to the live note while a version shows.
  const onPreviewKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented || !preview()) return;
    e.preventDefault();
    endPreview();
  };
  onMount(() => document.addEventListener("keydown", onPreviewKey));
  onCleanup(() => document.removeEventListener("keydown", onPreviewKey));

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
  /**
   * The tabs that show for the note as it is shown now, each with its index in the template: a
   * tab whose `when` is false is left out, and changes as fields are edited, before any save.
   */
  const visible = () => {
    const t = type();
    const n = note();
    return t && n ? visibleTabs(tabs(), whenNote(t, n.id, shown())) : [];
  };
  // The open tab, hidden by an edit, gives way to the first tab that shows.
  createEffect(() => {
    const v = visible();
    const i = untrack(tab);
    if (v.length && !v.some((x) => x.index === i)) setTab(v[0]!.index);
  });

  /** Apply an edit to the note, and save it after a pause (a draft waits for Create). */
  function edit(fn: (d: NoteDoc) => NoteDoc) {
    // A version on show is read-only: nothing edits or saves until the live note is back. Nor
    // while the conflict view holds the edits: what it compares is what Save my version sends.
    if (preview() || conflict()) return;
    setDoc(fn(doc()));
    autosave?.schedule();
  }

  function setField(f: FieldDef, v: Json | undefined) {
    edit((d) => (f.kind === "dict" ? applyDict(d, f, v) : applyEdits(d, { [f.name]: v })));
  }

  /** Several fields in one edit, such as an Outbox item's status and when it was sent. */
  function setFields(values: Record<string, Json | undefined>) {
    edit((d) => applyEdits(d, values));
  }

  /** The note a field editor belongs to. */
  const related: RelatedFields = {
    get type() {
      return type()?.name;
    },
  };

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
      const from = props.prefill;
      if (from?.from) {
        // Opened from a placeholder link: that link becomes a mention of the new note.
        try {
          await resolvePlaceholder(from.from, n.type, from.title, saved.id);
        } catch (e) {
          console.warn(`could not link the new note from ${from.from}`, e);
        }
      }
      navigate(`/notes/${saved.id}`);
    } catch (e) {
      setSaveError(message(e));
    } finally {
      setBusy(false);
    }
  }

  /** Whether the stored note is archived. */
  const archived = () => !props.newType && isArchived(note()?.note.fields);
  /** The note this one is a sync conflict copy of, if it is one. */
  const original = () => (props.newType ? undefined : conflictOf(note()?.note.fields));
  // The series an occurrence of a recurring event belongs to: its hidden `series_ref` (event-series E1).
  const seriesOf = () => {
    const r = props.newType ? undefined : doc().fields.series_ref;
    return typeof r === "string" && r ? r : undefined;
  };

  /** Archive the note, or take it out of the archive: a new version, after any pending save. */
  async function toggleArchive() {
    const n = note();
    if (!n || props.newType || busy()) return;
    const archiving = !isArchived(n.note.fields);
    setBusy(true);
    setSaveError(null);
    try {
      await autosave?.flush();
      const base = note()?.version ?? n.version;
      const saved = archiving ? await api.archiveNote(n.id, base) : await api.unarchiveNote(n.id, base);
      knowArchived(saved.id, isArchived(saved.note.fields));
      if (note()?.id !== saved.id) return;
      batch(() => {
        setNote(saved);
        // The edited note follows the stored one unless it has edits of its own, which the
        // next save sends over the new version.
        if (!autosave?.dirty()) setDoc(saved.note);
      });
    } catch (e) {
      setSaveError(message(e));
    } finally {
      setBusy(false);
    }
  }

  // The command palette archives the note the active pane shows, through this page.
  const owner = `${uid}${Math.random().toString(36).slice(2)}`;
  createEffect(() => {
    const n = note();
    if (n && n.id && !props.newType && isActive()) {
      setActiveNote({ owner, id: n.id, archived: archived(), toggle: toggleArchive });
    } else {
      clearActiveNote(owner);
    }
  });
  onCleanup(() => clearActiveNote(owner));

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
    showOutsideTitle();
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
      showOutsideTitle();
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
  const refs: FieldRefValues = { text: (name) => refText(type(), shown(), name) };

  const status = (): string => {
    if (props.newType) return missing().length ? `Required: ${missing().join(", ")}` : "";
    if (conflict()) return "Unsaved changes";
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
  // While a version shows, they show its values and cannot write.
  const write = (f: FieldDef, v: Json | undefined) => setField(f, v);
  const controls: ControlBinding = {
    type: () => type(),
    value: (f) => fieldValue(shown(), f),
    get set() {
      return preview() ? undefined : write;
    },
    get reason() {
      return preview() ? "Viewing an older version." : undefined;
    },
    noteId: () => note()?.id || undefined,
  };
  /** The editors remount when the note is replaced from outside, or a preview starts or ends. */
  const layoutKey = () => `${generation()}:${preview()?.version ?? ""}`;

  return (
    <ControlBindingContext.Provider value={controls}>
    <article
      class="note-view"
      classList={{ previewing: !!preview() }}
      data-save-state={saveState()}
      onFocusOut={flushOnLeave}
    >
      <Show when={preview()}>
        {(p) => (
          <div class="preview-banner" role="region" aria-label="Version preview">
            <span class="preview-banner-text">
              Viewing the version from <time datetime={new Date(p().at).toISOString()}>{when(p().at)}</time> by {authorText(p().author)}
            </span>
            <span class="preview-banner-actions">
              <button type="button" class="link" onClick={() => void restorePreview()}>
                Restore this version
              </button>
              <span class="preview-banner-action">
                <span aria-hidden="true">·</span>
                <button type="button" class="link" onClick={endPreview}>
                  Back to current
                </button>
              </span>
              <Show when={rightIsDrawer()}>
                <span class="preview-banner-action">
                  <span aria-hidden="true">·</span>
                  <button type="button" class="link" onClick={() => focusPanel("history")}>
                    Versions
                  </button>
                </span>
              </Show>
            </span>
            <Show when={saveError()}>
              <p class="error" role="alert">
                {saveError()}
              </p>
            </Show>
          </div>
        )}
      </Show>
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
                <Show when={props.newType}>
                  <p class="note-kind">New {type()?.label ?? n().type}</p>
                </Show>
                {/* The title is inline text, so it runs the full width before it wraps and the chip and
                    buttons follow its last word, moving to the next line only when they reach the edge. */}
                <div class="note-title-flow">
                  <h1 class="note-title">
                    <span
                      ref={(el) => (titleEl = el)}
                      id={`${uid}note-title`}
                      class="note-title-input"
                      role="textbox"
                      aria-label="Title"
                      aria-required="true"
                      aria-readonly={preview() || conflict() ? "true" : undefined}
                      contentEditable={preview() || conflict() ? false : "plaintext-only"}
                      spellcheck={false}
                      data-placeholder="Untitled"
                      onInput={(e) => {
                        const el = e.currentTarget;
                        const raw = el.textContent ?? "";
                        // A title is one line: a pasted line break becomes a space.
                        const text = raw.replace(/\s*[\r\n]+\s*/g, " ");
                        if (text !== raw) el.textContent = text;
                        // An emptied box keeps no stray node, so its placeholder shows.
                        if (text === "" && el.firstChild) el.replaceChildren();
                        setField(TITLE_FIELD, text === "" ? undefined : text);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          flush();
                        }
                      }}
                    />
                    {" "}
                  </h1>
                  <span class="note-title-tools">
                    <a
                      class="type-chip"
                      href={`/types/${n().type}`}
                      data-colour={type()?.colour ?? undefined}
                      title={`All ${type()?.plural ?? type()?.label ?? n().type}`}
                      aria-label={`Type: ${type()?.label ?? n().type}`}
                    >
                      <TypeIcon name={type()?.icon} size={14} />
                      <span>{type()?.label ?? n().type}</span>
                    </a>
                    <Show when={archived()}>
                      <span class="archived-chip" title="This note is archived" data-testid="archived-chip">
                        Archived
                      </span>
                    </Show>
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
                      <button
                        type="button"
                        class="icon-button history-toggle"
                        aria-label="History"
                        title="History"
                        aria-expanded={historyShown()}
                        aria-controls="history-panel"
                        onClick={toggleHistory}
                      >
                        <HistoryIcon />
                      </button>
                      <button
                        type="button"
                        class="icon-button archive-button"
                        aria-label={archived() ? "Unarchive" : "Archive"}
                        title={archived() ? "Unarchive note" : "Archive note"}
                        aria-pressed={archived()}
                        onClick={() => void toggleArchive()}
                        disabled={busy()}
                      >
                        <Show when={archived()} fallback={<ArchiveIcon />}>
                          <UnarchiveIcon />
                        </Show>
                      </button>
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
                  </span>
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
                <Show when={!props.newType && !preview() && !conflict()}>
                  <NoteOutboxActions
                    id={n().id}
                    type={n().type}
                    doc={doc()}
                    disabled={busy()}
                    setStatus={(status, extra) => {
                      setFields({ status, ...extra });
                      flush();
                    }}
                    onDelete={() => void remove()}
                  />
                </Show>
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
            <Show when={archived()}>
              <div class="archived-banner" role="region" aria-label="Archived">
                <ArchiveIcon />
                <span class="archived-banner-text">
                  <strong>Archived</strong>
                  <span class="muted"> · left out of search and lists</span>
                </span>
                <button type="button" class="link" onClick={() => void toggleArchive()} disabled={busy()}>
                  Unarchive
                </button>
              </div>
            </Show>
            <Show when={original()}>
              {(id) => (
                <div class="conflict-copy-banner" role="region" aria-label="Conflict copy">
                  <span class="conflict-copy-text">
                    <strong>Conflict copy</strong>
                    <span class="muted"> · sync kept another device's version as the original: </span>
                  </span>
                  <a href={`/notes/${id()}`}>{noteTitle(id()) ?? "Original note"}</a>
                </div>
              )}
            </Show>
            <Show when={seriesOf()}>
              {(id) => (
                <div class="series-banner" role="region" aria-label="Series">
                  <span class="muted">Part of </span>
                  <a href={`/notes/${id()}`} data-testid="series-link">
                    {noteTitle(id()) ?? "the series"}
                  </a>
                </div>
              )}
            </Show>
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
                    <Show when={visible().length > 1}>
                      <div class="tabs" role="tablist">
                        <For each={visible()}>
                          {(t) => (
                            <button
                              type="button"
                              role="tab"
                              id={`${uid}tab-${t.index}`}
                              aria-selected={tab() === t.index}
                              aria-controls={`${uid}tab-panel`}
                              onClick={() => {
                                flush();
                                setTab(t.index);
                              }}
                            >
                              {t.tab.title}
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                    {/* Keyed on the generation and the preview: a note replaced from outside, or a version
                        shown in its place, remounts the editors. */}
                    <Show when={layoutKey()} keyed>
                      {(_) => (
                        <PlaceholdersInert.Provider value={!!preview()}>
                        <fieldset class="preview-lock" disabled={!!preview()}>
                        <div
                          id={`${uid}tab-panel`}
                          role={visible().length > 1 ? "tabpanel" : undefined}
                          aria-labelledby={visible().length > 1 ? `${uid}tab-${tab()}` : undefined}
                          class="tab-panel"
                        >
                          <For each={tabs()[tab()]?.items ?? []}>
                            {(item) =>
                              "slot" in item ? (
                                // The heading is the title's editor; a title slot would repeat it.
                                item.slot === "title" ? null : <Slot name={item.slot} label={item.label} when={item.when} />
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
                        </fieldset>
                        </PlaceholdersInert.Provider>
                      )}
                    </Show>
                  </div>
                </div>
              }
            >
              {(c) => (
                <PlaceholdersInert.Provider value={true}>
                <ConflictView
                  type={type()}
                  mine={c().mine}
                  stored={c().stored.note}
                  busy={busy()}
                  onKeepStored={keepStored}
                  onSaveMine={() => void saveMine()}
                />
                </PlaceholdersInert.Provider>
              )}
            </Show>
          </ThisNote.Provider>
        )}
      </Show>
    </article>
    </ControlBindingContext.Provider>
  );

  /**
   * A field's value read-only: references by their notes' titles, files of an asset field as its
   * editor lists them (a single image as its thumbnail), anything else as text.
   */
  function Display(p: { field: FieldDef; value: Json | undefined }) {
    const ids = () => (Array.isArray(p.value) ? p.value : [p.value]).filter((x) => typeof x === "string");
    if (p.field.kind === "asset" && !(p.field.accept === "image" && !p.field.multiple)) {
      return <AssetInput field={p.field} value={p.value} label={p.field.label} />;
    }
    return (
      <Show when={p.value !== undefined} fallback={<span class="muted">—</span>}>
        <Show when={p.field.kind === "reference"} fallback={<FieldValue field={p.field} value={p.value} />}>
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
  function FieldEditor(p: { field: FieldDef; label: string; ns: string; bare?: boolean }): JSX.Element {
    const f = p.field;
    const id = `${p.ns}field-input-${f.name}`;
    if (f.computed) {
      return (
        <Show when={f.compound && f.kind !== "dict"} fallback={<Display field={f} value={fieldValue(shown(), f)} />}>
          <Blocks blocks={compoundBlock(shown(), f)?.blocks ?? []} />
        </Show>
      );
    }
    // A version on show: the same layout, nothing editable. Inputs sit in a disabled fieldset.
    if (preview()) {
      if (f.kind === "content") {
        return (
          <div class="content-readonly" role="document" aria-label={p.label}>
            <Blocks blocks={compoundBlock(shown(), f)?.blocks ?? []} scope={note()?.id ? `${note()!.id}/${f.name}` : undefined} />
          </div>
        );
      }
      if (f.kind === "ink") return <InkSurface label={p.label} block={compoundBlock(shown(), f)} />;
      if (f.kind === "canvas") return <CanvasSurface label={p.label} block={compoundBlock(shown(), f)} />;
    }
    switch (f.kind) {
      case "content":
        return (
          <ContentEditor
            label={p.label}
            blocks={compoundBlock(shown(), f)?.blocks ?? []}
            onChange={(blocks) => edit((d) => applyBlocks(d, f, blocks))}
            toggleScope={note()?.id ? `${note()!.id}/${f.name}` : undefined}
          />
        );
      case "ink":
        return (
          <InkSurface
            label={p.label}
            block={compoundBlock(shown(), f)}
            onChange={(block) => edit((d) => applyCompound(d, f, block))}
          />
        );
      case "canvas":
        return (
          <CanvasSurface
            label={p.label}
            block={compoundBlock(shown(), f)}
            onChange={(block) => edit((d) => applyCompound(d, f, block))}
          />
        );
    }
    if (hasInput(f)) {
      return <FieldInput
          field={f}
          value={fieldValue(shown(), f)}
          related={related}
          onChange={(v) => setField(f, v)}
          label={p.label}
          id={id}
          placeholder={p.bare ? p.label : "Empty"}
        />;
    }
    return (
      <Show when={f.compound && f.kind !== "dict"} fallback={<Display field={f} value={fieldValue(shown(), f)} />}>
        <Blocks blocks={compoundBlock(shown(), f)?.blocks ?? []} />
      </Show>
    );
  }

  /**
   * A property block: a field's editor in place. On the note page (`bare`) the label is hidden,
   * kept for screen readers and shown as the empty placeholder and a tooltip; the Properties
   * panel shows it.
   */
  function Property(p: { field: FieldDef; label: string; ns?: string; bare?: boolean }) {
    const ns = p.ns ?? uid;
    const f = p.field;
    const wide = f.kind === "content" || f.kind === "ink" || f.kind === "canvas" || f.kind === "location";
    return (
      <div
        class="field property"
        classList={{ wide, bare: !!p.bare, [`kind-${f.kind}`]: true }}
        data-field={f.name}
        data-testid={`field-${f.name}`}
        title={p.bare ? p.label : undefined}
      >
        <div class="field-label" classList={{ "visually-hidden": !!p.bare }}>
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
          <FieldEditor field={f} label={p.label} ns={ns} bare={p.bare} />
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
    const when = b.fields?.when;
    return <Slot name={slot} label={typeof label === "string" ? label : null} when={typeof when === "string" ? when : null} />;
  }

  /** Whether a slot with condition `when` shows for the note as it is shown now, as a tab's does. */
  function slotShows(when: string | null | undefined): boolean {
    if (!when) return true;
    const t = type();
    const n = note();
    return !t || !n || whenHolds(when, whenNote(t, n.id, shown()));
  }

  function Slot(p: { name: string; label: string | null; when?: string | null }) {
    const t = type();
    const f = t ? slotField(t, p.name) : undefined;
    // An Outbox item's status is shown, not chosen: the note page's Outbox buttons move it (outbox
    // X13). Beside it, "AI" when an agent drafted the item (outbox X14).
    if (t?.name === OUTBOX && p.name === "status") {
      return (
        <Show when={slotShows(p.when)}>
          <div class="field property outbox-status-field" data-field="status" data-testid="field-status">
            <span class="field-label">{p.label ?? f?.label ?? "Status"}</span>
            <OutboxChips fields={shown().fields} />
          </div>
        </Show>
      );
    }
    return (
      <Show when={slotShows(p.when)}>
        <Show when={f} fallback={<div class="field missing">Unknown field {p.name}</div>}>
          {(field) => <Property field={field()} label={p.label ?? field().label} bare />}
        </Show>
      </Show>
    );
  }
}
