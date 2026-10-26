import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createSignal, on } from "solid-js";
import { createStore, produce, reconcile, unwrap } from "solid-js/store";
import { ApiError, type FieldDef, type Json, type Row, type SchemaIssue, type TypePlan, api } from "../api";
import { FieldInput } from "../components/FieldInput";
import {
  EVENTS,
  FIELD_KINDS,
  type FieldKind,
  type FieldModel,
  type HandlerModel,
  LIST_SORT_BUILT_INS,
  type ListPageModel,
  ON_DELETE,
  PALETTE,
  type Place,
  RANGE_OF,
  type TypeModel,
  VALUE_KINDS,
  changeKind,
  emptyType,
  fieldDefOf,
  inverseChoices,
  isLinkList,
  isName,
  isOwnInverse,
  itemKind,
  itemPlace,
  makeLinkList,
  needsDefault,
  newField,
  optionsText as writeOptions,
  parseOptions,
  problems,
  relationsAt,
  renames,
  takesDefault,
  typeFromSchema,
  typeWcl,
} from "../defs/typeModel";
import { reloadDefinitions, taxonomies, typeDef, types } from "../session";
import { IconPicker } from "../components/IconPicker";
import { TemplateIcon, TrashIcon } from "../components/Icon";
import { ScriptField } from "../components/ScriptField";

export function errorText(e: unknown): string {
  if (e instanceof ApiError) {
    const issues = e.body.issues as { at?: string; message: string; file?: string }[] | undefined;
    if (issues?.length) return issues.map((i) => (i.at ? `${i.at}: ${i.message}` : i.message)).join("; ");
    const failures = e.body.failures as { issues: { at: string; message: string }[] }[] | undefined;
    if (failures?.length) {
      return failures.flatMap((f) => f.issues.map((i) => `${i.at}: ${i.message}`)).join("; ");
    }
  }
  return e instanceof Error ? e.message : String(e);
}

/**
 * What this version ignored when it loaded the type file, such as an attribute of the type or a
 * field that only a newer release knows. The type still works, so these do not stop a save, but
 * they show, so a typo is seen. Saving keeps them in the file as they were.
 */
export function TypeWarnings(props: { warnings: SchemaIssue[] }) {
  return (
    <Show when={props.warnings.length > 0}>
      <section class="type-warnings" aria-label="Warnings">
        <p class="muted">
          This version ignores {props.warnings.length === 1 ? "this part" : "these parts"} of the type file. The type
          still works. Saving here keeps {props.warnings.length === 1 ? "it" : "them"} in the file as {props.warnings.length === 1 ? "it is" : "they are"}.
        </p>
        <ul>
          <For each={props.warnings}>
            {(w) => (
              <li>
                {w.at}: {w.message}
              </li>
            )}
          </For>
        </ul>
      </section>
    </Show>
  );
}

/** The field as a note page would edit it, so a default is entered with the field's own editor. */
function defaultDef(f: FieldModel): FieldDef {
  return { ...fieldDefOf(f), name: "default", label: "Default", description: null, required: false, computed: false, hidden: false };
}

const KIND_LABELS: Record<FieldKind, string> = {
  string: "Text",
  int: "Whole number",
  float: "Number",
  bool: "Yes/no",
  date: "Date",
  datetime: "Date and time",
  time: "Time",
  date_range: "Date range",
  url: "URL",
  select: "Select",
  taxonomy: "Taxonomy",
  reference: "Reference",
  list: "List",
  dict: "Dictionary",
  location: "Location",
  content: "Content",
  asset: "Asset",
  ink: "Ink",
  canvas: "Canvas",
};

/** The Kind picker's preset for a list of links, which is a list (typeModel.ts, makeLinkList). */
const LINKS = "links";

/** What an item of a list is called in its Item kind picker. */
const ITEM_LABELS: Partial<Record<FieldKind, string>> = {
  taxonomy: "Taxonomy terms",
};

/** Kinds with settings of their own, which [`KindSettings`] shows. */
const SETTING_KINDS = new Set<string>(["select", "taxonomy", "reference", "date_range", "location", "asset", "list", "dict"]);

interface SettingsProps {
  def: FieldModel;
  /** Change the definition. Any preview is now out of date. */
  set: (fn: (x: FieldModel) => void) => void;
  place: Place;
  /** The element id of one of the definition's inputs. */
  id: (part: string) => string;
  /** The type being edited, offered as a reference target before it is saved. */
  typeName: string;
  /** The field this definition is, or is the item of; empty for a dict's key or other entries. */
  fieldName: string;
}

/** A picker of the kinds a list item, a dict key or a dict's other entries may be (types T1). */
function ValueKindSelect(props: { id: string; label?: string; value: FieldKind | null; onChange: (kind: FieldKind) => void; items?: boolean }) {
  return (
    <select id={props.id} aria-label={props.label} onChange={(e) => props.onChange(e.currentTarget.value as FieldKind)}>
      <For each={VALUE_KINDS}>
        {(k) => (
          <option value={k} selected={props.value === k}>
            {(props.items && ITEM_LABELS[k]) || KIND_LABELS[k]}
          </option>
        )}
      </For>
    </select>
  );
}

/**
 * The settings of a definition's kind (types T8): a select's options, a taxonomy, a reference's
 * target and relation, a date range's ends, a location's map, an asset's files, and what a list
 * or a dict holds, each item and key with these same settings of its own.
 */
function KindSettings(props: SettingsProps) {
  const f = () => props.def;
  const id = props.id;
  const set = (fn: (x: FieldModel) => void) => props.set(fn);
  const [optionsText, setOptionsText] = createSignal(writeOptions(f()));
  const targetFields = () =>
    types()
      .find((t) => t.name === f().target)
      ?.fields.filter((x) => x.kind === "reference") ?? [];
  // The relations this place allows, and the stored one even when it does not.
  const relations = () => {
    const ok = relationsAt(props.place);
    return ok.includes(f().relation) ? ok : [...ok, f().relation];
  };
  return (
    <>
      <Show when={f().kind === "select"}>
        <label for={id("options")}>Options, one per line, as value or value = Label</label>
        <textarea
          id={id("options")}
          rows={3}
          value={optionsText()}
          onInput={(e) => {
            setOptionsText(e.currentTarget.value);
            const parsed = parseOptions(e.currentTarget.value);
            set((x) => {
              x.options = parsed.options;
              x.option_labels = parsed.option_labels;
            });
          }}
        />
      </Show>
      <Show when={f().kind === "taxonomy"}>
        <label for={id("taxonomy")}>Taxonomy</label>
        <select
          id={id("taxonomy")}
          onChange={(e) =>
            set((x) => {
              x.taxonomy = e.currentTarget.value;
              x.default = undefined;
            })
          }
        >
          <option value="" selected={!f().taxonomy}>
            —
          </option>
          <For each={taxonomies()}>
            {(t) => (
              <option value={t.name} selected={f().taxonomy === t.name}>
                {t.label}
              </option>
            )}
          </For>
        </select>
      </Show>
      <Show when={f().kind === "reference"}>
        <label for={id("target")}>Target type</label>
        <select id={id("target")} onChange={(e) => set((x) => (x.target = e.currentTarget.value))}>
          <option value="" selected={!f().target}>
            —
          </option>
          <option value="*" selected={f().target === "*"}>
            Any type
          </option>
          <For each={[...new Set([...types().map((t) => t.name), ...(props.typeName ? [props.typeName] : [])])].sort()}>
            {(t) => (
              <option value={t} selected={f().target === t}>
                {t}
              </option>
            )}
          </For>
        </select>
        <label for={id("relation")}>Relation</label>
        <select id={id("relation")} onChange={(e) => set((x) => (x.relation = e.currentTarget.value as FieldModel["relation"]))}>
          <For each={relations()}>
            {(r) => (
              <option value={r} selected={f().relation === r}>
                {{ one_way: "One-way", two_way: "Two-way", peer: "Peer group" }[r]}
              </option>
            )}
          </For>
        </select>
        <Show when={props.place !== "field" && props.place !== "fieldItem"}>
          <p class="muted hint">Inside a dictionary or a nested list, a reference is one-way.</p>
        </Show>
        <Show when={f().relation === "two_way"}>
          <label for={id("inverse")}>Inverse field</label>
          <input
            id={id("inverse")}
            list={id("inverse-list")}
            value={f().inverse}
            onInput={(e) => set((x) => (x.inverse = e.currentTarget.value))}
          />
          <datalist id={id("inverse-list")}>
            <For each={inverseChoices(props.typeName, props.fieldName, f().target, targetFields())}>
              {(c) => <option value={c.value} label={c.label} />}
            </For>
          </datalist>
          <Show
            when={isOwnInverse(props.typeName, props.fieldName, f())}
            fallback={
              <p class="muted hint">
                The field on {f().target || "the target type"} that holds the other side. When it has no field by that
                name, it gets one on save: a two-way reference back that holds several notes.
              </p>
            }
          >
            <p class="muted hint">
              This field is its own other side: when a note lists another here, the other lists it back here too.
              Unlike a peer group, the notes it lists are not linked to each other.
            </p>
          </Show>
        </Show>
        <label for={id("on_delete")}>When the target is deleted</label>
        <select id={id("on_delete")} onChange={(e) => set((x) => (x.on_delete = e.currentTarget.value as FieldModel["on_delete"]))}>
          <For each={ON_DELETE}>
            {(o) => (
              <option value={o} selected={f().on_delete === o}>
                {{ unlink: "Unlink", restrict: "Refuse the delete", cascade: "Delete this note too" }[o]}
              </option>
            )}
          </For>
        </select>
      </Show>
      <Show when={f().kind === "date_range"}>
        <label for={id("of")}>Ends are</label>
        <select
          id={id("of")}
          onChange={(e) =>
            set((x) => {
              x.of = e.currentTarget.value as FieldModel["of"];
              x.default = undefined;
            })
          }
        >
          <For each={RANGE_OF}>
            {(o) => (
              <option value={o.value} selected={f().of === o.value}>
                {o.label}
              </option>
            )}
          </For>
        </select>
      </Show>
      <Show when={f().kind === "location"}>
        <label for={id("map")}>Map</label>
        <select id={id("map")} onChange={(e) => set((x) => (x.map = e.currentTarget.value as FieldModel["map"]))}>
          <option value="geo" selected={f().map === "geo"}>
            Real-world map
          </option>
          <option value="image" selected={f().map === "image"}>
            Image map
          </option>
        </select>
        <Show when={f().map === "image"}>
          <label for={id("image")}>Map image</label>
          <FieldInput
            field={{ ...fieldDefOf(f()), kind: "asset", multiple: false, listed: false, accept: "any", label: "Map image" }}
            value={f().image || undefined}
            onChange={(v) => set((x) => (x.image = typeof v === "string" ? v : ""))}
            label="Map image"
            id={id("image")}
          />
        </Show>
      </Show>
      <Show when={f().kind === "asset"}>
        <label for={id("accept")}>Takes</label>
        <select id={id("accept")} onChange={(e) => set((x) => (x.accept = e.currentTarget.value === "image" ? "image" : "any"))}>
          <option value="any" selected={f().accept !== "image"}>
            Any file
          </option>
          <option value="image" selected={f().accept === "image"}>
            Images only
          </option>
        </select>
      </Show>
      <Show when={f().kind === "list"}>
        <label for={id("items")}>Item kind</label>
        <ValueKindSelect
          id={id("items")}
          items
          value={itemKind(f())}
          onChange={(k) =>
            set((x) => {
              x.item ??= newField(x.name);
              changeKind(x.item, k);
              if (!relationsAt(itemPlace(props.place, k)).includes(x.item.relation)) x.item.relation = "one_way";
              x.default = undefined;
            })
          }
        />
        <Show when={f().item && SETTING_KINDS.has(f().item!.kind) ? f().item : null}>
          {(item) => (
            <div class="nested-settings" data-testid="item-settings" role="group" aria-label="Item settings">
              <div class="form-grid">
                <KindSettings
                  def={item()}
                  set={(fn) =>
                    set((x) => {
                      if (x.item) fn(x.item);
                      x.default = undefined;
                    })
                  }
                  place={itemPlace(props.place, item().kind)}
                  id={(part) => id(`item-${part}`)}
                  typeName={props.typeName}
                  fieldName={props.fieldName}
                />
              </div>
            </div>
          )}
        </Show>
      </Show>
      <Show when={f().kind === "dict"}>
        <DictSettings {...props} />
      </Show>
    </>
  );
}

/** A dict's settings: its keys, each a row with settings of its own, and what other keys may hold. */
function DictSettings(props: SettingsProps) {
  const f = () => props.def;
  const id = props.id;
  const values = () => (f().values === null ? "none" : f().values === "any" ? "any" : "one");
  const valuesDef = () => {
    const v = f().values;
    return v && v !== "any" ? v : null;
  };
  function addKey() {
    props.set((x) => {
      let n = x.keys.length + 1;
      while (x.keys.some((k) => k.name === `key_${n}`)) n++;
      x.keys.push(newField(`key_${n}`));
    });
  }
  function moveKey(key: number, by: number) {
    props.set((x) => {
      const i = x.keys.findIndex((k) => k.key === key);
      const j = i + by;
      if (i < 0 || j < 0 || j >= x.keys.length) return;
      const [k] = x.keys.splice(i, 1);
      x.keys.splice(j, 0, k!);
    });
  }
  return (
    <>
      <span>Keys</span>
      <div class="dict-keys" role="group" aria-label="Keys">
        <For each={f().keys} fallback={<p class="muted">No keys declared.</p>}>
          {(k, i) => <KeyRow {...props} keyDef={k} index={i()} count={f().keys.length} move={moveKey} />}
        </For>
        <button type="button" onClick={addKey}>
          Add key
        </button>
      </div>
      <label for={id("values")}>Other keys</label>
      <select
        id={id("values")}
        onChange={(e) =>
          props.set((x) => {
            const v = e.currentTarget.value;
            x.values = v === "none" ? null : v === "any" ? "any" : x.values && x.values !== "any" ? x.values : newField(x.name);
          })
        }
      >
        <option value="none" selected={values() === "none"}>
          None: only the keys above
        </option>
        <option value="any" selected={values() === "any"}>
          Any kind, chosen for each entry
        </option>
        <option value="one" selected={values() === "one"}>
          One kind
        </option>
      </select>
      <Show when={valuesDef()}>
        {(v) => (
          <>
            <label for={id("values-kind")}>Kind of other keys</label>
            <ValueKindSelect
              id={id("values-kind")}
              value={v().kind}
              onChange={(k) =>
                props.set((x) => {
                  if (x.values && x.values !== "any") {
                    changeKind(x.values, k);
                    x.values.relation = "one_way";
                  }
                })
              }
            />
            <Show when={SETTING_KINDS.has(v().kind)}>
              <div class="nested-settings" data-testid="values-settings" role="group" aria-label="Other keys settings">
                <div class="form-grid">
                  <KindSettings
                    def={v()}
                    set={(fn) =>
                      props.set((x) => {
                        if (x.values && x.values !== "any") fn(x.values);
                      })
                    }
                    place="values"
                    id={(part) => id(`values-${part}`)}
                    typeName={props.typeName}
                    fieldName=""
                  />
                </div>
              </div>
            </Show>
          </>
        )}
      </Show>
    </>
  );
}

/** One declared key of a dict: its name, kind, label and whether it is required, and its settings underneath. */
function KeyRow(props: SettingsProps & { keyDef: FieldModel; index: number; count: number; move: (key: number, by: number) => void }) {
  const k = () => props.keyDef;
  const key = k().key;
  const set = (fn: (x: FieldModel) => void) =>
    props.set((x) => {
      const found = x.keys.find((y) => y.key === key);
      if (found) fn(found);
    });
  const id = (part: string) => props.id(`key${key}-${part}`);
  const [open, setOpen] = createSignal(k().original === null && SETTING_KINDS.has(k().kind));
  const who = () => k().name || "key";
  return (
    <div class="dict-key" classList={{ open: open() }} data-testid={`key-row-${props.index}`}>
      <div class="dict-key-row">
        <button type="button" aria-label={`Move key ${who()} up`} disabled={props.index === 0} onClick={() => props.move(key, -1)}>
          ↑
        </button>
        <button
          type="button"
          aria-label={`Move key ${who()} down`}
          disabled={props.index === props.count - 1}
          onClick={() => props.move(key, 1)}
        >
          ↓
        </button>
        <input aria-label="Key name" value={k().name} onInput={(e) => set((x) => (x.name = e.currentTarget.value))} />
        <ValueKindSelect
          id={id("kind")}
          label="Key kind"
          value={k().kind}
          onChange={(kind) =>
            set((x) => {
              changeKind(x, kind);
              x.relation = "one_way";
              if (SETTING_KINDS.has(kind)) setOpen(true);
            })
          }
        />
        <input
          aria-label="Key label"
          value={k().label}
          placeholder={k().name}
          onInput={(e) => set((x) => (x.label = e.currentTarget.value))}
        />
        <label class="dict-key-flag">
          <input type="checkbox" aria-label="Key required" checked={k().required} onChange={(e) => set((x) => (x.required = e.currentTarget.checked))} />
          <span>Required</span>
        </label>
        <button
          type="button"
          aria-label={`Settings for key ${who()}`}
          aria-expanded={open()}
          title="More settings"
          onClick={() => setOpen(!open())}
        >
          ⚙
        </button>
        <button
          type="button"
          class="danger"
          aria-label={`Remove key ${who()}`}
          title="Remove key"
          onClick={() => props.set((x) => void x.keys.splice(x.keys.findIndex((y) => y.key === key), 1))}
        >
          ✕
        </button>
      </div>
      <Show when={open()}>
        <div class="nested-settings" role="group" aria-label={`Key ${who()} settings`}>
          <div class="form-grid">
            <label for={id("description")}>Description</label>
            <input id={id("description")} value={k().description} onInput={(e) => set((x) => (x.description = e.currentTarget.value))} />
            <KindSettings def={k()} set={set} place="key" id={id} typeName={props.typeName} fieldName="" />
          </div>
        </div>
      </Show>
    </div>
  );
}

/**
 * The type editor (D3, D6): the type's own items and its fields, each kind with its options. A new
 * type is created at once. A change to a stored type is previewed first, listing every note it
 * edits, and applied from that preview.
 */
export function TypeEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const isNew = () => params.type === undefined;
  const [name, setName] = createSignal("");
  const [model, setModel] = createStore<TypeModel>(emptyType());
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [plan, setPlan] = createSignal<TypePlan | null>(null);
  // Why the type was not deleted, and whether it was its notes that stopped it.
  const [deleteError, setDeleteError] = createSignal<{ message: string; hasNotes: boolean } | null>(null);
  // A preview the server refused: why, and the notes the change would reach.
  const [refused, setRefused] = createSignal<{ message: string; notes: Row[]; total: number } | null>(null);
  // The script and function a change runs over every note of the type, if any (D6).
  const [migrationScript, setMigrationScript] = createSignal("");
  const [migrationFunction, setMigrationFunction] = createSignal("");
  // What this version ignored when it loaded the stored type file.
  const [warnings, setWarnings] = createSignal<SchemaIssue[]>([]);

  createEffect(
    on(
      () => params.type,
      async (type) => {
        setLoaded(false);
        setLoadError(null);
        setPlan(null);
        setRefused(null);
        setSaveError(null);
        setDeleteError(null);
        setWarnings([]);
        if (type === undefined) {
          setName("");
          setModel(reconcile(emptyType()));
          setLoaded(true);
          return;
        }
        try {
          const schema = await api.schema();
          const t = schema.types.find((x) => x.name === type);
          if (!t) {
            setLoadError(`There is no type ${type}.`);
            return;
          }
          setName(type);
          setWarnings(t.warnings ?? []);
          setModel(reconcile(typeFromSchema(t)));
          setLoaded(true);
        } catch (e) {
          setLoadError(errorText(e));
        }
      },
    ),
  );

  // What the list page may sort by: the built-in note fields, then the type's own that hold one
  // value a page can order by, and the stored sort field even when it is no longer offered.
  const listSortOptions = () => {
    const builtIn = LIST_SORT_BUILT_INS.map((n) => ({ value: n, text: n[0]!.toUpperCase() + n.slice(1) }));
    const own = model.fields
      .filter((f) => isName(f.name) && !["content", "canvas", "ink", "location", "dict"].includes(f.kind))
      .map((f) => ({ value: f.name, text: f.label ? `${f.label} (${f.name})` : f.name }));
    const all = [...builtIn, ...own];
    const s = model.listPage.sortField;
    if (s && !all.some((o) => o.value === s)) all.push({ value: s, text: s });
    return all;
  };

  /** Change the model. Any preview is now out of date. */
  function edit(fn: (m: TypeModel) => void) {
    setModel(produce(fn));
    setPlan(null);
    setRefused(null);
    setSaveError(null);
  }
  const editHandler = (index: number, fn: (h: HandlerModel) => void) =>
    edit((m) => {
      const h = m.handlers[index];
      if (h) fn(h);
    });

  function moveHandler(index: number, by: number) {
    edit((m) => {
      const j = index + by;
      if (j < 0 || j >= m.handlers.length) return;
      const [h] = m.handlers.splice(index, 1);
      m.handlers.splice(j, 0, h!);
    });
  }

  function migration() {
    const script = migrationScript();
    if (!script) return undefined;
    const fn = migrationFunction().trim();
    return fn ? { script, function: fn } : { script };
  }

  const editField = (key: number, fn: (f: FieldModel) => void) =>
    edit((m) => {
      const f = m.fields.find((x) => x.key === key);
      if (f) fn(f);
    });

  function addField() {
    edit((m) => {
      let n = m.fields.length + 1;
      while (m.fields.some((f) => f.name === `field_${n}`)) n++;
      m.fields.push(newField(`field_${n}`));
    });
  }

  function moveField(key: number, by: number) {
    edit((m) => {
      const i = m.fields.findIndex((f) => f.key === key);
      const j = i + by;
      if (i < 0 || j < 0 || j >= m.fields.length) return;
      const [f] = m.fields.splice(i, 1);
      m.fields.splice(j, 0, f!);
    });
  }

  const issues = () => [...(isNew() && !isName(name()) ? ["The type needs a name: a-z, 0-9 and _."] : []), ...problems(model)];

  /** A new type gets a default template placing every field. */
  function withDefaultTemplate(m: TypeModel): TypeModel {
    const copy = structuredClone(unwrap(m));
    const tab = copy.templates.find((t) => t.name === "default")?.tabs[0];
    if (tab) {
      for (const f of copy.fields) {
        if (!tab.items.some((i) => "slot" in i && i.slot === f.name)) tab.items.push({ slot: f.name, label: null });
      }
    }
    return copy;
  }

  async function create() {
    setBusy(true);
    setSaveError(null);
    try {
      await api.putType(name(), typeWcl(withDefaultTemplate(model)));
      await reloadDefinitions();
      navigate(`/types/${name()}`);
    } catch (e) {
      setSaveError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function preview() {
    setBusy(true);
    setSaveError(null);
    setPlan(null);
    setRefused(null);
    try {
      setPlan(await api.changeType(name(), typeWcl(model), renames(model), undefined, migration()));
    } catch (e) {
      if (e instanceof ApiError && (e.status === 400 || e.status === 422)) {
        // Show what the change would reach even though it cannot be applied as it stands.
        try {
          const page = await api.query(`from ${name()} sort title asc select title`, null, 200);
          setRefused({ message: errorText(e), notes: page.rows, total: page.total });
        } catch {
          setRefused({ message: errorText(e), notes: [], total: 0 });
        }
      } else {
        setSaveError(errorText(e));
      }
    } finally {
      setBusy(false);
    }
  }


  async function remove() {
    if (!confirm(`Delete type ${name()}? Its type file and templates go too.`)) return;
    setBusy(true);
    setDeleteError(null);
    try {
      await api.deleteType(name());
      await reloadDefinitions();
      navigate("/settings/types");
    } catch (e) {
      if (e instanceof ApiError && e.body.error === "type_in_use") {
        const n = Number(e.body.count ?? 0);
        setDeleteError({
          message: `${model.label || name()} still has ${n} ${n === 1 ? "note" : "notes"}. Delete ${n === 1 ? "it" : "them"} before deleting the type.`,
          hasNotes: true,
        });
      } else {
        setDeleteError({ message: errorText(e), hasNotes: false });
      }
    } finally {
      setBusy(false);
    }
  }

  async function apply() {
    const p = plan();
    if (!p || needsDefault(model).length > 0) return;
    setBusy(true);
    setSaveError(null);
    try {
      await api.changeType(name(), typeWcl(model), renames(model), p.plan, migration());
      await reloadDefinitions();
      navigate(`/types/${name()}`);
    } catch (e) {
      setSaveError(errorText(e));
      if (e instanceof ApiError && e.status === 409) setPlan(null);
    } finally {
      setBusy(false);
    }
  }

  function describeEdit(e: { op: string; [key: string]: Json }): string {
    if (e.op === "rename") return `rename ${e.from} to ${e.to}`;
    if (e.op === "default") return `set ${e.field} to ${JSON.stringify(e.value)}`;
    if (e.op === "migrate") return `run ${e.script}.${e.function}`;
    return e.op;
  }

  function FieldRow(props: { field: FieldModel; index: number }) {
    const f = () => props.field;
    const key = f().key;
    const set = (fn: (x: FieldModel) => void) => editField(key, fn);
    // A field added in this session opens with its settings showing.
    const [open, setOpen] = createSignal(f().original === null);
    const id = (part: string) => `f${key}-${part}`;
    return (
      <tbody class="field-row" classList={{ open: open() }} data-testid={`field-row-${props.index}`}>
        <tr>
          <td class="field-order">
            <button
              type="button"
              aria-label={`Move ${f().name || "field"} up`}
              onClick={() => moveField(key, -1)}
              disabled={props.index === 0}
            >
              ↑
            </button>
            <button
              type="button"
              aria-label={`Move ${f().name || "field"} down`}
              onClick={() => moveField(key, 1)}
              disabled={props.index === model.fields.length - 1}
            >
              ↓
            </button>
          </td>
          <td>
            <input
              id={id("name")}
              aria-label="Name"
              value={f().name}
              onInput={(e) => set((x) => (x.name = e.currentTarget.value))}
            />
            <Show when={f().original !== null && f().original !== f().name}>
              <span class="muted field-was"> was {f().original}</span>
            </Show>
            <Show when={f().original === null && !isNew()}>
              <span class="badge">new</span>
            </Show>
          </td>
          <td>
            <select
              id={id("kind")}
              aria-label="Kind"
              onChange={(e) => {
                const v = e.currentTarget.value;
                set((x) => (v === LINKS ? makeLinkList(x) : changeKind(x, v as FieldKind)));
              }}
            >
              <For each={FIELD_KINDS}>
                {(k) => (
                  <>
                    <option value={k} selected={f().kind === k && !(k === "list" && isLinkList(f()))}>
                      {KIND_LABELS[k]}
                    </option>
                    {/* A list of dicts of a name and a URL, ready made. */}
                    <Show when={k === "list"}>
                      <option value={LINKS} selected={isLinkList(f())}>
                        List of links
                      </option>
                    </Show>
                  </>
                )}
              </For>
            </select>
          </td>
          <td>
            <input
              id={id("label")}
              aria-label="Label"
              value={f().label}
              placeholder={f().name}
              onInput={(e) => set((x) => (x.label = e.currentTarget.value))}
            />
          </td>
          <td class="field-flag">
            <input
              type="checkbox"
              aria-label="Required"
              checked={f().required}
              disabled={f().computed || f().hidden}
              onChange={(e) => set((x) => (x.required = e.currentTarget.checked))}
            />
          </td>
          <td class="field-flag">
            <input
              type="checkbox"
              aria-label="Computed"
              checked={f().computed}
              disabled={f().required}
              onChange={(e) => set((x) => (x.computed = e.currentTarget.checked))}
            />
          </td>
          <td class="field-flag">
            <input
              type="checkbox"
              aria-label="Hidden"
              title="Keep this field off the note page and the properties sidebar"
              checked={f().hidden}
              disabled={f().required}
              onChange={(e) => set((x) => (x.hidden = e.currentTarget.checked))}
            />
          </td>
          <td class="field-actions">
            <button
              type="button"
              aria-label={`Settings for ${f().name || "field"}`}
              aria-expanded={open()}
              title="More settings"
              onClick={() => setOpen(!open())}
            >
              ⚙
            </button>
            <button
              type="button"
              class="danger"
              aria-label={`Remove ${f().name || "field"}`}
              title="Remove field"
              onClick={() => edit((m) => void m.fields.splice(m.fields.findIndex((x) => x.key === key), 1))}
            >
              ✕
            </button>
          </td>
        </tr>
        <Show when={open()}>
          <tr class="field-details">
            <td colSpan={8}>
              <div class="form-grid">
                <label for={id("description")}>Description</label>
                <input
                  id={id("description")}
                  value={f().description}
                  onInput={(e) => set((x) => (x.description = e.currentTarget.value))}
                />
                <KindSettings def={f()} set={set} place="field" id={id} typeName={name()} fieldName={f().name} />
                <Show when={takesDefault(f())}>
                  <label for={id("default")}>Default</label>
                  <Show when={`${f().kind}/${itemKind(f())}/${f().of}/${f().taxonomy}/${f().item?.taxonomy}/${f().item?.of}`} keyed>
                    {(_kind) => (
                    <FieldInput
                      field={defaultDef(f())}
                      fixedTerms
                      value={f().default}
                      onChange={(v) => set((x) => (x.default = v))}
                      label={`Default for ${f().name}`}
                      id={id("default")}
                    />
                    )}
                  </Show>
                </Show>
              </div>
            </td>
          </tr>
        </Show>
      </tbody>
    );
  }

  function HandlerCard(props: { handler: HandlerModel; index: number }) {
    const h = () => props.handler;
    const i = () => props.index;
    const set = (fn: (x: HandlerModel) => void) => editHandler(i(), fn);
    const id = (part: string) => `h${i()}-${part}`;
    const lines = (text: string) =>
      text
        .split("\n")
        .map((l) => l.trim())
        .filter(Boolean);
    return (
      <fieldset class="field-card" data-testid={`handler-card-${i()}`}>
        <legend>
          {h().event} <span class="muted">{h().script ? `${h().script}.${h().function || h().event}` : ""}</span>
        </legend>
        <div class="form-grid">
          <label for={id("event")}>Event</label>
          <select id={id("event")} onChange={(e) => set((x) => (x.event = e.currentTarget.value))}>
            <For each={EVENTS}>
              {(ev) => (
                <option value={ev} selected={h().event === ev}>
                  {ev}
                </option>
              )}
            </For>
          </select>
          <label for={id("script")}>Script</label>
          <ScriptField id={id("script")} value={h().script} onChange={(name) => set((x) => (x.script = name))} />
          <label for={id("function")}>Function</label>
          <input
            id={id("function")}
            value={h().function ?? ""}
            placeholder={h().event}
            onInput={(e) => set((x) => (x.function = e.currentTarget.value.trim() || null))}
          />
          <Show when={h().event.startsWith("after_")}>
            <label for={id("domains")}>Allowed hosts, one per line</label>
            <textarea
              id={id("domains")}
              rows={2}
              value={h().allow_domains.join("\n")}
              onChange={(e) => set((x) => (x.allow_domains = lines(e.currentTarget.value)))}
            />
            <label for={id("secrets")}>Secrets, one per line</label>
            <textarea
              id={id("secrets")}
              rows={2}
              value={h().secrets.join("\n")}
              onChange={(e) => set((x) => (x.secrets = lines(e.currentTarget.value)))}
            />
          </Show>
        </div>
        <div class="actions">
          <button type="button" onClick={() => moveHandler(i(), -1)} disabled={i() === 0}>
            Move up
          </button>
          <button type="button" onClick={() => moveHandler(i(), 1)} disabled={i() === model.handlers.length - 1}>
            Move down
          </button>
          <button type="button" class="danger" onClick={() => edit((m) => void m.handlers.splice(i(), 1))}>
            Remove handler
          </button>
        </div>
      </fieldset>
    );
  }

  return (
    <section class="def-editor">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={loaded()}>
        <header class="note-header">
          <div class="note-heading">
            <p class="note-kind">{isNew() ? "New type" : "Edit type"}</p>
            <div class="note-title-row def-title-row">
              <h1>{model.label || name() || "Untitled type"}</h1>
              <Show when={!isNew()}>
                <A
                  href={`/settings/types/${name()}/templates`}
                  class="icon-button"
                  aria-label="Templates"
                  title="Edit templates"
                >
                  <TemplateIcon />
                </A>
                <Show when={!typeDef(name())?.builtin}>
                  <button
                    type="button"
                    class="icon-button danger"
                    aria-label="Delete type"
                    title="Delete type"
                    disabled={busy()}
                    onClick={() => void remove()}
                  >
                    <TrashIcon />
                  </button>
                </Show>
              </Show>
            </div>
          </div>
        </header>
        <Show when={deleteError()}>
          {(err) => (
            <p class="error" role="alert">
              {err().message}{" "}
              <Show when={err().hasNotes}>
                <A href={`/types/${name()}`}>Show its notes</A>
              </Show>
            </p>
          )}
        </Show>
        <div class="form-grid">
          <Show when={isNew()}>
            <label for="type-name">Name</label>
            <input
              id="type-name"
              value={name()}
              placeholder="project"
              onInput={(e) => {
                setName(e.currentTarget.value);
                setSaveError(null);
              }}
            />
          </Show>
          <label for="type-label">Label</label>
          <input id="type-label" value={model.label} onInput={(e) => edit((m) => (m.label = e.currentTarget.value))} />
          <label for="type-plural">Plural</label>
          <input
            id="type-plural"
            value={model.plural}
            placeholder={model.label}
            onInput={(e) => edit((m) => (m.plural = e.currentTarget.value))}
          />
          <label for="type-description">Description</label>
          <input
            id="type-description"
            value={model.description}
            onInput={(e) => edit((m) => (m.description = e.currentTarget.value))}
          />
          <label for="type-icon">Icon</label>
          <IconPicker id="type-icon" value={model.icon} onChange={(name) => edit((m) => (m.icon = name))} />
          <label for="type-colour">Colour</label>
          <select id="type-colour" onChange={(e) => edit((m) => (m.colour = e.currentTarget.value))}>
            <option value="" selected={!model.colour}>
              —
            </option>
            <For each={PALETTE}>
              {(c) => (
                <option value={c} selected={model.colour === c}>
                  {c}
                </option>
              )}
            </For>
          </select>
        </div>

        <h2>Fields</h2>
        <Show when={model.fields.length > 0} fallback={<p class="muted">No fields yet. Every note has a title.</p>}>
          <div class="field-grid-wrap">
            <table class="field-grid" aria-label="Fields">
              <thead>
                <tr>
                  <th scope="col">
                    <span class="visually-hidden">Order</span>
                  </th>
                  <th scope="col">Name</th>
                  <th scope="col">Kind</th>
                  <th scope="col">Label</th>
                  <th scope="col">Required</th>
                  <th scope="col">Computed</th>
                  <th scope="col">Hidden</th>
                  <th scope="col">
                    <span class="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <For each={model.fields}>{(f, i) => <FieldRow field={f} index={i()} />}</For>
            </table>
          </div>
        </Show>
        <button type="button" onClick={addField}>
          Add field
        </button>

        <h2>Handlers</h2>
        <p class="muted">
          Scripts that run when a note of this type is created, updated or deleted. Handlers for one event run in
          this order. Pick a script for each, or start a new one, and edit it right here.
        </p>
        <For each={model.handlers} fallback={<p class="muted">No handlers.</p>}>
          {(h, i) => <HandlerCard handler={h} index={i()} />}
        </For>
        <button
          type="button"
          onClick={() =>
            edit((m) =>
              void m.handlers.push({ event: "before_update", script: "", function: null, allow_domains: [], secrets: [] }),
            )
          }
        >
          Add handler
        </button>

        <h2>List page</h2>
        <p class="muted">
          How the type's page of notes shows them when nobody has chosen otherwise in their browser. Left on Default,
          it shows cards, newest change first, 50 to a page.
        </p>
        <div class="form-grid" data-testid="list-page-settings">
          <label for="list-page-view">View</label>
          <select
            id="list-page-view"
            onChange={(e) => edit((m) => (m.listPage.view = e.currentTarget.value as ListPageModel["view"]))}
          >
            <For
              each={[
                ["", "Default (cards)"],
                ["cards", "Cards"],
                ["list", "List"],
                ["details", "Details"],
              ]}
            >
              {([value, text]) => (
                <option value={value} selected={model.listPage.view === value}>
                  {text}
                </option>
              )}
            </For>
          </select>
          <label for="list-page-sort">Sort by</label>
          <div class="list-page-sort">
            <select id="list-page-sort" onChange={(e) => edit((m) => (m.listPage.sortField = e.currentTarget.value))}>
              <option value="" selected={!model.listPage.sortField}>
                Default (updated, newest first)
              </option>
              <For each={listSortOptions()}>
                {(o) => (
                  <option value={o.value} selected={model.listPage.sortField === o.value}>
                    {o.text}
                  </option>
                )}
              </For>
            </select>
            <select
              aria-label="Sort direction"
              disabled={!model.listPage.sortField}
              onChange={(e) => edit((m) => (m.listPage.sortDir = e.currentTarget.value === "asc" ? "asc" : "desc"))}
            >
              <option value="asc" selected={model.listPage.sortDir === "asc"}>
                Ascending
              </option>
              <option value="desc" selected={model.listPage.sortDir === "desc"}>
                Descending
              </option>
            </select>
          </div>
          <label for="list-page-size">Page size</label>
          <input
            id="list-page-size"
            type="number"
            min="10"
            max="500"
            step="1"
            placeholder="50"
            value={model.listPage.pageSize}
            onInput={(e) => edit((m) => (m.listPage.pageSize = e.currentTarget.value))}
          />
        </div>

        <Show when={!isNew()}>
          <h2>Migration</h2>
          <p class="muted">
            Optionally run a script function over every note of this type when the change is applied.
          </p>
          <div class="form-grid">
            <label for="migration-script">Migration script</label>
            <ScriptField
              id="migration-script"
              value={migrationScript()}
              noneLabel="None"
              onChange={(name) => {
                setMigrationScript(name);
                setPlan(null);
              }}
            />
            <Show when={migrationScript()}>
              <label for="migration-function">Migration function</label>
              <input
                id="migration-function"
                value={migrationFunction()}
                placeholder="migrate"
                onInput={(e) => {
                  setMigrationFunction(e.currentTarget.value);
                  setPlan(null);
                }}
              />
            </Show>
          </div>
        </Show>

        <TypeWarnings warnings={warnings()} />

        <div class="save-bar">
          <Show when={issues().length > 0}>
            <ul class="error" role="alert" aria-label="Problems">
              <For each={issues()}>{(p) => <li>{p}</li>}</For>
            </ul>
          </Show>
          <Show when={saveError()}>
            <p class="error" role="alert">
              {saveError()}
            </p>
          </Show>
          <Show
            when={isNew()}
            fallback={
              <button type="button" class="primary" disabled={busy() || issues().length > 0} onClick={() => void preview()}>
                Preview changes
              </button>
            }
          >
            <button type="button" class="primary" disabled={busy() || issues().length > 0} onClick={() => void create()}>
              Create type
            </button>
          </Show>
        </div>

        <Show when={refused()}>
          {(r) => (
            <section class="preview" aria-label="Change preview">
              <h2>This change cannot be applied yet</h2>
              <p class="error" role="alert">
                {r().message}
              </p>
              <Show when={needsDefault(model).length > 0}>
                <p>
                  Give {needsDefault(model).join(", ")} a default. Every note below would receive it.
                </p>
              </Show>
              <p>
                {r().total} {r().total === 1 ? "note" : "notes"} of this type would change.
              </p>
              <ul class="preview-notes" aria-label="Affected notes">
                <For each={r().notes}>
                  {(n) => (
                    <li>
                      <A href={`/notes/${n.id}`}>{n.title || "Untitled"}</A>
                    </li>
                  )}
                </For>
              </ul>
            </section>
          )}
        </Show>

        <Show when={plan()}>
          {(p) => (
            <section class="preview" aria-label="Change preview">
              <h2>Preview</h2>
              <Show when={p().added.length}>
                <p>Adds {p().added.join(", ")}.</p>
              </Show>
              <Show when={p().removed.length}>
                <p>Removes {p().removed.join(", ")}. Notes keep the data and are flagged.</p>
              </Show>
              <Show when={p().renamed.length}>
                <p>Renames {p().renamed.map((r) => `${r.from} to ${r.to}`).join(", ")}.</p>
              </Show>
              <p>
                {p().changes.length} {p().changes.length === 1 ? "note changes" : "notes change"}.
              </p>
              <ul class="preview-notes" aria-label="Affected notes">
                <For each={p().changes}>
                  {(c) => (
                    <li>
                      <A href={`/notes/${c.id}`}>{c.title || "Untitled"}</A>
                      <span class="muted"> — {c.edits.map(describeEdit).join("; ")}</span>
                    </li>
                  )}
                </For>
              </ul>
              <Show when={p().flagged.length}>
                <p>{p().flagged.length} notes will hold data the type no longer describes.</p>
              </Show>
              <button
                type="button"
                class="primary"
                disabled={busy() || needsDefault(model).length > 0}
                onClick={() => void apply()}
              >
                Apply changes
              </button>
            </section>
          )}
        </Show>
      </Show>
    </section>
  );
}
