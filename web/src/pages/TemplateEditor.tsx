import { A, useParams } from "@solidjs/router";
import { For, Show, createEffect, createSignal, on } from "solid-js";
import { createStore, produce, reconcile, unwrap } from "solid-js/store";
import { type Json, api } from "../api";
import { type ControlBinding, ControlBindingContext } from "../controls/binding";
import { type DraggedBlock } from "../editor/blockTools";
import { ContentEditor, type EditorHandle, type TemplateEditing } from "../editor/ContentEditor";
import { docToItems, itemsToDoc } from "../editor/doc";
import type { SlotContext } from "../editor/SlotHolder";
import { type TemplateModel, templateWcl, unplaced } from "../defs/typeModel";
import { reloadDefinitions, typeDef } from "../session";
import { errorText } from "./TypeEditor";
import { whenProblem } from "../when";

interface FieldInfo {
  name: string;
  label: string;
  kind: string;
}

/** The only templates a type has: the note page, and cards. Both can have tabs. */
const TEMPLATES: [string, string][] = [
  ["default", "Default"],
  ["card", "Card"],
];

/**
 * The template editor (D14): a type's templates edited as the note page shows them, a tab at a
 * time, with the same inline block editor notes use. Fixed blocks are typed in place; `/` inserts
 * blocks and property blocks (a field's slot), `{{` a field's value inside text. Blocks are
 * dragged by their handles, onto another tab too, and a template is saved on its own.
 */
export function TemplateEditor() {
  const params = useParams();
  const typeName = () => params.type ?? "";
  const [templates, setTemplates] = createStore<TemplateModel[]>([]);
  const [fields, setFields] = createSignal<FieldInfo[]>([]);
  const [label, setLabel] = createSignal("");
  const [selected, setSelected] = createSignal("default");
  const [tab, setTab] = createSignal(0);
  const [stored, setStored] = createSignal<Set<string>>(new Set());
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The type's templates have arrived; until then the picker would act on nothing.
  const [loaded, setLoaded] = createSignal(false);
  // Bumped when a tab's items change outside its editor, so the editor starts again from them.
  const [revision, setRevision] = createSignal(0);
  const [dragged, setDragged] = createSignal<DraggedBlock | null>(null);
  let handle: EditorHandle | null = null;

  createEffect(
    on(typeName, async (name) => {
      setLoadError(null);
      setLoaded(false);
      try {
        const schema = await api.schema();
        const t = schema.types.find((x) => x.name === name);
        if (!t) {
          setLoadError(`There is no type ${name}.`);
          return;
        }
        setLabel(t.label);
        setFields(t.fields.map((f) => ({ name: String(f.name), label: String(f.label ?? f.name), kind: String(f.kind) })));
        setTemplates(reconcile(structuredClone(t.templates)));
        setStored(new Set(t.templates.map((x) => x.name)));
        setSelected(t.templates.some((x) => x.name === "default") ? "default" : (t.templates[0]?.name ?? "default"));
        setTab(0);
        setRevision(revision() + 1);
        setLoaded(true);
      } catch (e) {
        setLoadError(errorText(e));
      }
    }),
  );

  const index = () => templates.findIndex((t) => t.name === selected());
  const current = (): TemplateModel | undefined => templates[index()];
  /** What is wrong with the open tab's `when`, as the server would say when the template is saved. */
  const whenError = (): string | null => {
    const w = current()?.tabs[tab()]?.when?.trim();
    return w ? whenProblem(w, fields()) : null;
  };

  function edit(fn: (t: TemplateModel) => void) {
    const i = index();
    if (i < 0) return;
    setTemplates(i, produce(fn));
    setStatus(null);
    setError(null);
  }

  /** Change the items of tabs other than through their editor, and show the result. */
  function editTabs(fn: (t: TemplateModel) => void, show?: number) {
    edit(fn);
    if (show !== undefined) setTab(show);
    setRevision(revision() + 1);
  }

  function fieldOf(name: string): FieldInfo {
    if (name === "title") return { name, label: "Title", kind: "string" };
    return fields().find((f) => f.name === name) ?? { name, label: name, kind: "unknown" };
  }

  /** Move the block at `pos` in the open tab's editor to the end of tab `to`, and show that tab when `show`. */
  function moveToTab(pos: number, to: number, show = false) {
    const editor = handle?.editor;
    const node = editor?.state.doc.nodeAt(pos);
    if (!editor || !node || to === tab()) return;
    const [item] = docToItems({ type: "doc", content: [node.toJSON()] });
    // Deleting it updates this tab's items through the editor.
    editor.view.dispatch(editor.state.tr.delete(pos, pos + node.nodeSize));
    if (!item) return;
    editTabs((t) => void t.tabs[to]?.items.push(item), show ? to : undefined);
  }

  const slots: SlotContext = {
    describe: (name) => {
      const f = fieldOf(name);
      return { label: f.label, kind: f.kind, known: f.kind !== "unknown" };
    },
    tabs: () => current()?.tabs.map((t) => t.title) ?? [],
    tab,
    moveToTab,
  };

  const templateEditing: TemplateEditing = {
    slots,
    unplaced: () => {
      const t = current();
      return t ? unplaced(t, fields()).map((n) => ({ name: n, label: fieldOf(n).label })) : [];
    },
    fields: () => ["title", ...fields().map((f) => f.name)].map((n) => ({ name: n, label: fieldOf(n).label })),
  };

  function moveTab(by: number) {
    const i = tab();
    const j = i + by;
    if (j < 0 || j >= (current()?.tabs.length ?? 0)) return;
    editTabs((t) => {
      const [x] = t.tabs.splice(i, 1);
      t.tabs.splice(j, 0, x!);
    }, j);
  }

  function addTab() {
    editTabs((t) => void t.tabs.push({ title: `Tab ${t.tabs.length + 1}`, items: [] }), current()?.tabs.length ?? 0);
  }

  function removeTab() {
    const i = tab();
    editTabs((t) => {
      if (t.tabs.length > 1) t.tabs.splice(i, 1);
    }, Math.max(0, i - 1));
  }

  /** Start the card template from the first tab of `default`, as cards show until it exists. */
  function addCard() {
    const first = templates.find((t) => t.name === "default")?.tabs[0];
    const tabs = first ? [structuredClone(unwrap(first))] : [{ title: "Card", items: [{ slot: "title", label: null }] }];
    setTemplates(templates.length, { name: "card", label: null, tabs });
    setError(null);
  }

  async function save() {
    const t = current();
    if (!t) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await api.putTemplate(typeName(), t.name, templateWcl(t));
      setStored(new Set([...stored(), t.name]));
      await reloadDefinitions();
      setStatus(`Saved the ${t.name} template.`);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    const t = current();
    if (!t || t.name === "default") return;
    setBusy(true);
    setError(null);
    try {
      if (stored().has(t.name)) await api.deleteTemplate(typeName(), t.name);
      setTemplates((list) => list.filter((x) => x.name !== t.name));
      setSelected("default");
      setTab(0);
      setRevision(revision() + 1);
      await reloadDefinitions();
      setStatus(`Deleted the ${t.name} template.`);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  // Controls in the template are tried out on sample values kept here, never saved.
  const [sample, setSample] = createSignal<Record<string, Json>>({});
  const controls: ControlBinding = {
    type: () => typeDef(typeName()),
    value: (f) => sample()[f.name],
    set: (f, v) => {
      const next = { ...sample() };
      if (v === undefined) delete next[f.name];
      else next[f.name] = v;
      setSample(next);
    },
    preview: true,
  };

  return (
    <ControlBindingContext.Provider value={controls}>
    <section class="def-editor template-editor">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <header class="note-header">
        <div>
          <p class="note-kind">Templates</p>
          <h1>{label() || typeName()}</h1>
        </div>
        <div class="actions">
          <A href={`/settings/types/${typeName()}/edit`}>Edit fields</A>
        </div>
      </header>

      <div class="template-picker" role="group" aria-label="Templates">
        <For each={TEMPLATES}>
          {([name, label]) => (
            <button
              type="button"
              disabled={!loaded()}
              aria-pressed={selected() === name}
              onClick={() => {
                if (!templates.some((t) => t.name === name)) addCard();
                setSelected(name);
                setTab(0);
                setRevision(revision() + 1);
                setStatus(null);
              }}
            >
              {label}
            </button>
          )}
        </For>
        <Show when={!templates.some((t) => t.name === "card")}>
          <span class="muted">Cards show the first tab of Default until you give them a card template.</span>
        </Show>
      </div>

      <Show when={current()}>
        {(t) => (
          <>
            <div class="form-grid">
              <label for="template-label">Template label</label>
              <input
                id="template-label"
                value={t().label ?? ""}
                onInput={(e) => edit((x) => (x.label = e.currentTarget.value || null))}
              />
              <label for="tab-title">Tab title</label>
              <input
                id="tab-title"
                value={t().tabs[tab()]?.title ?? ""}
                onInput={(e) =>
                  edit((x) => {
                    const tb = x.tabs[tab()];
                    if (tb) tb.title = e.currentTarget.value;
                  })
                }
              />
              <label for="tab-when">Show this tab when</label>
              <div class="tab-when">
                <input
                  id="tab-when"
                  class="mono"
                  placeholder='Always. Or a condition, such as status == "Active"'
                  value={t().tabs[tab()]?.when ?? ""}
                  aria-invalid={whenError() ? true : undefined}
                  aria-describedby="tab-when-help"
                  onInput={(e) =>
                    edit((x) => {
                      const tb = x.tabs[tab()];
                      if (tb) tb.when = e.currentTarget.value || null;
                    })
                  }
                />
                <Show
                  when={whenError()}
                  fallback={
                    <p id="tab-when-help" class="muted">
                      Written as a query's where clause, on this note's fields. Empty shows the tab on every note.
                    </p>
                  }
                >
                  <p id="tab-when-help" class="error" role="alert">
                    {whenError()}
                  </p>
                </Show>
              </div>
            </div>
            <div class="actions">
              <button type="button" onClick={addTab}>
                Add tab
              </button>
              <button type="button" onClick={() => moveTab(-1)} disabled={tab() === 0}>
                Move tab left
              </button>
              <button type="button" onClick={() => moveTab(1)} disabled={tab() >= t().tabs.length - 1}>
                Move tab right
              </button>
              <button type="button" onClick={removeTab} disabled={t().tabs.length < 2}>
                Remove tab
              </button>
            </div>

            <div class="template-canvas note-view" aria-label={`${t().name} template`}>
              <div class="tabs" role="tablist">
                <For each={t().tabs}>
                  {(tb, i) => (
                    <button
                      type="button"
                      role="tab"
                      aria-selected={tab() === i()}
                      title={tb.when ? `Shows when ${tb.when}` : undefined}
                      classList={{ "drop-target": dragged() !== null && tab() !== i() }}
                      onClick={() => setTab(i())}
                      onDragOver={(e) => {
                        if (dragged()) e.preventDefault();
                      }}
                      onDrop={(e) => {
                        e.preventDefault();
                        const d = dragged();
                        setDragged(null);
                        if (d) moveToTab(d.pos, i(), true);
                      }}
                    >
                      {tb.title || "Untitled tab"}
                    </button>
                  )}
                </For>
              </div>
              <p class="template-hint muted">
                Type <kbd>/</kbd> to add a block, a property or a control, and <kbd>{"{{"}</kbd> to show a field's value in text. Drag
                a block by its handle, onto another tab too.
              </p>
              {/* A fresh editor per template, tab and outside change; its own edits keep it. */}
              <Show when={`${t().name}:${tab()}:${revision()}`} keyed>
                {(_) => (
                  <div class="tab-panel" role="tabpanel">
                    <ContentEditor
                      label={`${t().tabs[tab()]?.title || "Tab"} content`}
                      blocks={[]}
                      onChange={() => {}}
                      doc={itemsToDoc(unwrap(t().tabs[tab()]?.items ?? []))}
                      onDoc={(d) => {
                        const items = docToItems(d);
                        edit((x) => {
                          const tb = x.tabs[tab()];
                          if (tb) tb.items = items;
                        });
                      }}
                      template={templateEditing}
                      onReady={(h) => (handle = h)}
                      onDrag={setDragged}
                    />
                  </div>
                )}
              </Show>
            </div>

            <div class="save-bar">
              <Show when={error()}>
                <p class="error" role="alert">
                  {error()}
                </p>
              </Show>
              <Show when={status()}>
                <p role="status">{status()}</p>
              </Show>
              <div class="actions">
                <button type="button" class="primary" disabled={busy()} onClick={() => void save()}>
                  Save template
                </button>
                <Show when={t().name !== "default"}>
                  <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
                    Delete template
                  </button>
                </Show>
              </div>
            </div>
          </>
        )}
      </Show>
    </section>
    </ControlBindingContext.Provider>
  );
}
