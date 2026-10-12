import { A, useParams } from "@solidjs/router";
import { For, Show, createEffect, createSignal, on } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { type Block, api } from "../api";
import { BlockView } from "../components/Render";
import { type TemplateItemModel, type TemplateModel, templateWcl, unplaced } from "../defs/typeModel";
import { reloadDefinitions } from "../session";
import { errorText } from "./TypeEditor";

interface FieldInfo {
  name: string;
  label: string;
  kind: string;
}

const STANDARD = ["default", "card", "detail"];
const TEMPLATE_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

function itemName(item: TemplateItemModel, index: number): string {
  return "slot" in item ? item.slot : `${item.block.kind} block ${index + 1}`;
}

/**
 * The template editor (D14): a type's templates laid out as the note page shows them, a tab at a
 * time. Fields and fixed blocks are placed, reordered and moved between tabs by dragging or with
 * each item's controls, and a template is saved on its own.
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
  const [newName, setNewName] = createSignal("");
  const [toPlace, setToPlace] = createSignal("");
  let dragging: { tab: number; index: number } | null = null;

  createEffect(
    on(typeName, async (name) => {
      setLoadError(null);
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
      } catch (e) {
        setLoadError(errorText(e));
      }
    }),
  );

  const index = () => templates.findIndex((t) => t.name === selected());
  const current = (): TemplateModel | undefined => templates[index()];

  function edit(fn: (t: TemplateModel) => void) {
    const i = index();
    if (i < 0) return;
    setTemplates(i, produce(fn));
    setStatus(null);
    setError(null);
  }

  function fieldOf(name: string): FieldInfo {
    if (name === "title") return { name, label: "Title", kind: "string" };
    return fields().find((f) => f.name === name) ?? { name, label: name, kind: "unknown" };
  }

  function move(from: { tab: number; index: number }, toTab: number, toIndex: number) {
    edit((t) => {
      const source = t.tabs[from.tab];
      const dest = t.tabs[toTab];
      if (!source || !dest) return;
      const [item] = source.items.splice(from.index, 1);
      if (!item) return;
      const at = Math.max(0, Math.min(toIndex, dest.items.length));
      dest.items.splice(at, 0, item);
    });
  }

  function moveTab(by: number) {
    const i = tab();
    const j = i + by;
    edit((t) => {
      if (j < 0 || j >= t.tabs.length) return;
      const [x] = t.tabs.splice(i, 1);
      t.tabs.splice(j, 0, x!);
    });
    if (j >= 0 && j < (current()?.tabs.length ?? 0)) setTab(j);
  }

  function addTab() {
    edit((t) => void t.tabs.push({ title: `Tab ${t.tabs.length + 1}`, items: [] }));
    setTab((current()?.tabs.length ?? 1) - 1);
  }

  function removeTab() {
    const i = tab();
    edit((t) => {
      if (t.tabs.length > 1) t.tabs.splice(i, 1);
    });
    setTab(Math.max(0, i - 1));
  }

  function placeField() {
    const name = toPlace();
    if (!name) return;
    edit((t) => void t.tabs[tab()]?.items.push({ slot: name, label: null }));
    setToPlace("");
  }

  function addBlock(kind: string) {
    const block: Block =
      kind === "heading"
        ? { kind: "heading", fields: { level: 2, text: "Heading" } }
        : { kind: "paragraph", fields: { text: "Text shown on every note" } };
    edit((t) => void t.tabs[tab()]?.items.push({ block }));
  }

  function newTemplate() {
    const name = newName().trim();
    if (!TEMPLATE_NAME.test(name) || templates.some((t) => t.name === name)) {
      setError(`${JSON.stringify(name)} is not a free template name.`);
      return;
    }
    setTemplates(templates.length, { name, label: null, tabs: [{ title: "Main", items: [{ slot: "title", label: null }] }] });
    setSelected(name);
    setTab(0);
    setNewName("");
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
      await reloadDefinitions();
      setStatus(`Deleted the ${t.name} template.`);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  function Item(props: { item: TemplateItemModel; index: number }) {
    const name = () => itemName(props.item, props.index);
    const tabs = () => current()?.tabs ?? [];
    return (
      <div
        class="template-item"
        draggable="true"
        data-testid={`template-item-${name()}`}
        onDragStart={(e) => {
          dragging = { tab: tab(), index: props.index };
          e.dataTransfer?.setData("text/plain", name());
        }}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (dragging) move(dragging, tab(), props.index);
          dragging = null;
        }}
      >
        <div class="template-item-body">
          <Show
            when={"slot" in props.item ? props.item : null}
            fallback={
              <Show when={"block" in props.item ? props.item.block : null}>
                {(block) => (
                  <>
                    <BlockView block={block()} />
                    <Show when={block().kind === "paragraph" || block().kind === "heading"}>
                      <input
                        aria-label={`Text of ${name()}`}
                        value={String(block().fields?.text ?? "")}
                        onInput={(e) =>
                          edit((t) => {
                            const it = t.tabs[tab()]?.items[props.index];
                            if (it && "block" in it) it.block.fields = { ...it.block.fields, text: e.currentTarget.value };
                          })
                        }
                      />
                    </Show>
                  </>
                )}
              </Show>
            }
          >
            {(slot) => {
              const f = () => fieldOf(slot().slot);
              return (
                <div class="field">
                  <div class="field-label">{slot().label ?? f().label}</div>
                  <div class="field-value">
                    <span class="slot-sample">{f().kind === "unknown" ? "Unknown field" : `${f().kind} field ${f().name}`}</span>
                    <input
                      aria-label={`Label for ${slot().slot}`}
                      placeholder={f().label}
                      value={slot().label ?? ""}
                      onInput={(e) =>
                        edit((t) => {
                          const it = t.tabs[tab()]?.items[props.index];
                          if (it && "slot" in it) it.label = e.currentTarget.value || null;
                        })
                      }
                    />
                  </div>
                </div>
              );
            }}
          </Show>
        </div>
        <div class="template-item-controls">
          <span class="drag-handle" aria-hidden="true">
            ⠿
          </span>
          <button
            type="button"
            aria-label={`Move ${name()} up`}
            disabled={props.index === 0}
            onClick={() => move({ tab: tab(), index: props.index }, tab(), props.index - 1)}
          >
            ↑
          </button>
          <button
            type="button"
            aria-label={`Move ${name()} down`}
            onClick={() => move({ tab: tab(), index: props.index }, tab(), props.index + 1)}
          >
            ↓
          </button>
          <Show when={tabs().length > 1}>
            <select
              aria-label={`Move ${name()} to tab`}
              onChange={(e) => {
                const to = Number(e.currentTarget.value);
                if (to !== tab()) move({ tab: tab(), index: props.index }, to, Number.MAX_SAFE_INTEGER);
              }}
            >
              <For each={tabs()}>
                {(t, i) => (
                  <option value={i()} selected={i() === tab()}>
                    {t.title}
                  </option>
                )}
              </For>
            </select>
          </Show>
          <button
            type="button"
            aria-label={`Remove ${name()}`}
            onClick={() => edit((t) => void t.tabs[tab()]?.items.splice(props.index, 1))}
          >
            ×
          </button>
        </div>
      </div>
    );
  }

  return (
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
          <A href={`/types/${typeName()}/edit`}>Edit fields</A>
        </div>
      </header>

      <div class="template-picker" role="group" aria-label="Templates">
        <For each={templates}>
          {(t) => (
            <button
              type="button"
              aria-pressed={selected() === t.name}
              onClick={() => {
                setSelected(t.name);
                setTab(0);
                setStatus(null);
              }}
            >
              {t.name}
            </button>
          )}
        </For>
        <input
          aria-label="New template name"
          list="template-names"
          placeholder="card"
          value={newName()}
          onInput={(e) => setNewName(e.currentTarget.value)}
        />
        <datalist id="template-names">
          <For each={STANDARD.filter((n) => !templates.some((t) => t.name === n))}>{(n) => <option value={n} />}</For>
        </datalist>
        <button type="button" onClick={newTemplate}>
          Add template
        </button>
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
                      onClick={() => setTab(i())}
                      onDragOver={(e) => e.preventDefault()}
                      onDrop={(e) => {
                        e.preventDefault();
                        if (dragging) move(dragging, i(), Number.MAX_SAFE_INTEGER);
                        dragging = null;
                        setTab(i());
                      }}
                    >
                      {tb.title || "Untitled tab"}
                    </button>
                  )}
                </For>
              </div>
              <div
                class="tab-panel"
                role="tabpanel"
                onDragOver={(e) => e.preventDefault()}
                onDrop={(e) => {
                  e.preventDefault();
                  if (dragging) move(dragging, tab(), Number.MAX_SAFE_INTEGER);
                  dragging = null;
                }}
              >
                <For each={t().tabs[tab()]?.items ?? []} fallback={<p class="muted">Drop fields here.</p>}>
                  {(item, i) => <Item item={item} index={i()} />}
                </For>
              </div>
            </div>

            <div class="actions">
              <select aria-label="Field to place" onChange={(e) => setToPlace(e.currentTarget.value)}>
                <option value="" selected={toPlace() === ""}>
                  Choose a field…
                </option>
                <For each={unplaced(t(), fields())}>
                  {(n) => (
                    <option value={n} selected={toPlace() === n}>
                      {fieldOf(n).label}
                    </option>
                  )}
                </For>
              </select>
              <button type="button" onClick={placeField} disabled={!toPlace()}>
                Place field
              </button>
              <button type="button" onClick={() => addBlock("heading")}>
                Add heading
              </button>
              <button type="button" onClick={() => addBlock("paragraph")}>
                Add paragraph
              </button>
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
  );
}
