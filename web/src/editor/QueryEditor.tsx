import { Index, Show, createEffect, createMemo, createSignal, createUniqueId, onMount } from "solid-js";
import { type TypeDef, api } from "../api";
import { ChipPicker, typeSuggestions } from "../components/ChipPicker";
import {
  type Condition,
  type Group,
  type Item,
  OPS,
  type Op,
  type Operand,
  type Order,
  type QueryModel,
  type Sort,
  VALUE_KINDS,
  type ValueKind,
  nameList,
  newCondition,
  numberOperand,
  operandOfKind,
  valueKind,
  withGroupItem,
  withOp,
} from "./queryModel";

type Mode = "text" | "builder";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * A query, edited as text or in the visual builder (D8). Both edit the same text: the server
 * turns the text into the builder's model and the model back into text. The text changes only
 * when someone edits it, so a query opened in the builder and left alone keeps every byte. A
 * query the builder cannot show opens as text, with a notice saying why.
 */
export function QueryEditor(props: { value: string; onChange: (text: string) => void; types: TypeDef[] }) {
  const [mode, setMode] = createSignal<Mode>("text");
  const [model, setModel] = createSignal<QueryModel | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // Set once someone types or picks a mode, so the opening parse never overrides them.
  let touched = false;
  let seq = 0;

  /** Show `text` in the builder when it can, else say why it stays text. */
  async function tryBuilder(text: string, opening: boolean) {
    let next: QueryModel | null = null;
    let why: string | null = null;
    try {
      const r = await api.parseQuery(text);
      next = r.model;
      why = r.builder_error;
    } catch (e) {
      why = message(e);
    }
    if ((opening && touched) || props.value !== text) return;
    if (next) {
      setModel(next);
      setError(null);
      setNotice(null);
      setMode("builder");
    } else {
      setNotice(why ?? "the builder cannot show this query");
      setMode("text");
    }
  }

  onMount(() => void tryBuilder(props.value, true));

  // Only the latest edit's text lands, and only onto the text it was made from: one that arrives
  // after someone typed in text mode is dropped rather than written over their typing.
  function edit(next: QueryModel) {
    setModel(next);
    const n = ++seq;
    const base = props.value;
    api.queryText(next).then(
      (text) => {
        if (n !== seq || props.value !== base) return;
        setError(null);
        if (text !== props.value) props.onChange(text);
      },
      (e) => {
        if (n === seq) setError(message(e));
      },
    );
  }

  return (
    <div class="query-editor">
      <div class="query-editor-modes" role="group" aria-label="Query editor">
        <span>Query</span>
        <button
          type="button"
          aria-pressed={mode() === "builder"}
          onClick={() => {
            touched = true;
            if (mode() !== "builder") void tryBuilder(props.value, false);
          }}
        >
          Builder
        </button>
        <button
          type="button"
          aria-pressed={mode() === "text"}
          onClick={() => {
            touched = true;
            setError(null);
            setMode("text");
          }}
        >
          Text
        </button>
      </div>
      <Show when={mode() === "text" && notice()}>
        {(n) => (
          <p class="query-editor-notice" role="status" data-testid="builder-notice">
            Shown as text: {n()}
          </p>
        )}
      </Show>
      <Show
        when={mode() === "builder" && model()}
        fallback={
          <textarea
            aria-label="Query"
            rows={2}
            spellcheck={false}
            value={props.value}
            onInput={(e) => {
              touched = true;
              props.onChange(e.currentTarget.value);
            }}
          />
        }
      >
        {(m) => <Builder model={m()} onChange={edit} types={props.types} />}
      </Show>
      <Show when={mode() === "builder" && error()}>
        {(e) => (
          <p class="query-editor-error" role="alert">
            Not a query yet, so the text is unchanged: {e()}
          </p>
        )}
      </Show>
    </div>
  );
}

function Builder(props: { model: QueryModel; onChange: (m: QueryModel) => void; types: TypeDef[] }) {
  const fieldList = createUniqueId();
  const set = <K extends keyof QueryModel>(key: K, value: QueryModel[K]) => props.onChange({ ...props.model, [key]: value });
  const typesLabel = createUniqueId();
  const label = (name: string) => props.types.find((t) => t.name === name)?.label ?? name;
  const fieldNames = createMemo(() => {
    const from = props.model.from;
    const types = from.length ? props.types.filter((t) => from.includes(t.name)) : props.types;
    return [...new Set(types.flatMap((t) => t.fields.map((f) => f.name)))].sort();
  });
  const setSort = (i: number, next: Sort | null) => {
    const sort = props.model.sort.slice();
    if (next) sort[i] = next;
    else sort.splice(i, 1);
    set("sort", sort);
  };

  return (
    <div class="query-builder" role="group" aria-label="Query builder">
      <datalist id={fieldList}>
        <Index each={fieldNames()}>{(f) => <option value={f()} />}</Index>
      </datalist>
      <fieldset class="query-builder-from">
        <legend id={typesLabel}>Types</legend>
        <ChipPicker
          label="Types"
          labelledBy={typesLabel}
          values={props.model.from}
          chipText={label}
          chipIcon={(name) => props.types.find((t) => t.name === name)?.icon ?? null}
          suggest={(text) => typeSuggestions(props.types, text)}
          placeholder="Find a type"
          onChange={(from) => set("from", from)}
        />
        <Show when={props.model.from.length === 0}>
          <span class="query-builder-hint">Every type</span>
        </Show>
      </fieldset>

      <fieldset class="query-builder-where">
        <legend>Conditions</legend>
        <Show
          when={props.model.filter}
          fallback={
            <button
              type="button"
              onClick={() => set("filter", { join: "all", items: [{ condition: newCondition() }] })}
            >
              Add condition
            </button>
          }
        >
          {(g) => <GroupEditor group={g()} onChange={(next) => set("filter", next)} fieldList={fieldList} top />}
        </Show>
      </fieldset>

      <label>
        Search text
        <input
          type="text"
          value={props.model.search ?? ""}
          onInput={(e) => set("search", e.currentTarget.value === "" ? null : e.currentTarget.value)}
        />
      </label>

      <fieldset class="query-builder-sort">
        <legend>Sort</legend>
        <Index each={props.model.sort}>
          {(s, i) => (
            <div class="query-builder-row" data-testid="sort-key">
              <label>
                Sort field
                <input
                  type="text"
                  list={fieldList}
                  value={s().field}
                  onInput={(e) => setSort(i, { ...s(), field: e.currentTarget.value })}
                />
              </label>
              <label>
                Order
                <select
                  value={s().order}
                  onChange={(e) => setSort(i, { ...s(), order: e.currentTarget.value as Order })}
                >
                  <option value="default">default</option>
                  <option value="asc">ascending</option>
                  <option value="desc">descending</option>
                </select>
              </label>
              <button type="button" aria-label={`Remove sort ${i + 1}`} onClick={() => setSort(i, null)}>
                Remove
              </button>
            </div>
          )}
        </Index>
        <button type="button" onClick={() => set("sort", [...props.model.sort, { field: "", order: "asc" }])}>
          Add sort
        </button>
      </fieldset>

      <div class="query-builder-row">
        <label>
          Group by
          <input
            type="text"
            list={fieldList}
            value={props.model.group_by ?? ""}
            onInput={(e) => set("group_by", e.currentTarget.value === "" ? null : e.currentTarget.value)}
          />
        </label>
        <label>
          Fields to return
          <input
            type="text"
            value={props.model.select.join(", ")}
            placeholder="all"
            onChange={(e) => set("select", nameList(e.currentTarget.value))}
          />
        </label>
        <label>
          Limit
          <input
            type="number"
            min={0}
            step={1}
            value={props.model.limit ?? ""}
            onInput={(e) => {
              const t = e.currentTarget.value.trim();
              if (t === "") return set("limit", null);
              const n = Number(t);
              if (Number.isSafeInteger(n) && n >= 0) set("limit", n);
            }}
          />
        </label>
      </div>
    </div>
  );
}

/** A group of conditions. `onChange(null)` removes it: the last item of a group took it along. */
function GroupEditor(props: { group: Group; onChange: (g: Group | null) => void; fieldList: string; top?: boolean }) {
  const setItem = (i: number, next: Item | null) => {
    const g = withGroupItem(props.group, i, next);
    props.onChange(g.items.length ? g : null);
  };
  const add = (item: Item) => props.onChange({ ...props.group, items: [...props.group.items, item] });

  return (
    <div class="query-builder-group" data-testid="condition-group">
      <label>
        Match
        <select
          value={props.group.join}
          onChange={(e) => props.onChange({ ...props.group, join: e.currentTarget.value as Group["join"] })}
        >
          <option value="all">all of these</option>
          <option value="any">any of these</option>
        </select>
      </label>
      <Index each={props.group.items}>
        {(item, i) => (
          <Show
            when={"condition" in item() && (item() as { condition: Condition }).condition}
            fallback={
              <GroupEditor
                group={(item() as { group: Group }).group}
                onChange={(g) => setItem(i, g && { group: g })}
                fieldList={props.fieldList}
              />
            }
          >
            {(c) => (
              <ConditionRow
                condition={c()}
                onChange={(next) => setItem(i, next && { condition: next })}
                fieldList={props.fieldList}
              />
            )}
          </Show>
        )}
      </Index>
      <div class="query-builder-row">
        <button type="button" onClick={() => add({ condition: newCondition() })}>
          Add condition
        </button>
        <button type="button" onClick={() => add({ group: { join: "any", items: [{ condition: newCondition() }] } })}>
          Add group
        </button>
        <Show when={!props.top}>
          <button type="button" onClick={() => props.onChange(null)}>
            Remove group
          </button>
        </Show>
      </div>
    </div>
  );
}

/** A float that happens to be whole still shows its point, so it stays a float when edited. */
function numberText(v: Operand): string {
  if (v.kind === "float" && Number.isInteger(v.value)) return v.value.toFixed(1);
  return "value" in v ? String(v.value) : "";
}

const sameOperand = (a: Operand | null, b: Operand) =>
  a !== null && a.kind === b.kind && (a as { value: number }).value === (b as { value: number }).value;

/**
 * A number value. What is typed stays as typed while it means the stored number, so `2.` or `1e`
 * part-way through typing is not rewritten under the cursor.
 */
function NumberInput(props: { value: Operand; onChange: (v: Operand) => void }) {
  let el!: HTMLInputElement;
  createEffect(() => {
    const v = props.value;
    if (!sameOperand(numberOperand(el.value), v)) el.value = numberText(v);
  });
  return (
    <input
      ref={el}
      type="text"
      inputmode="decimal"
      onInput={(e) => {
        const n = numberOperand(e.currentTarget.value);
        if (n) props.onChange(n);
      }}
    />
  );
}

function ConditionRow(props: { condition: Condition; onChange: (c: Condition | null) => void; fieldList: string }) {
  const c = () => props.condition;
  const setValue = (value: Operand) => props.onChange({ ...c(), value });

  return (
    <div class="query-builder-row" data-testid="condition">
      <label>
        Field
        <input
          type="text"
          list={props.fieldList}
          value={c().field}
          onInput={(e) => props.onChange({ ...c(), field: e.currentTarget.value })}
        />
      </label>
      <label>
        Operator
        <select value={c().op} onChange={(e) => props.onChange(withOp(c(), e.currentTarget.value as Op))}>
          <Index each={OPS}>{(o) => <option value={o()[0]}>{o()[1]}</option>}</Index>
        </select>
      </label>
      <Show when={c().value}>
        {(v) => (
          <>
            <label>
              Value type
              <select
                value={valueKind(v())}
                onChange={(e) => setValue(operandOfKind(e.currentTarget.value as ValueKind, v()))}
              >
                <Index each={VALUE_KINDS}>{(k) => <option value={k()[0]}>{k()[1]}</option>}</Index>
              </select>
            </label>
            <Show when={v().kind === "str"}>
              <label>
                Value
                <input
                  type="text"
                  value={(v() as { value: string }).value}
                  onInput={(e) => setValue({ kind: "str", value: e.currentTarget.value })}
                />
              </label>
            </Show>
            <Show when={valueKind(v()) === "number"}>
              <label>
                Value
                <NumberInput value={v()} onChange={setValue} />
              </label>
            </Show>
            <Show when={v().kind === "bool"}>
              <label>
                Value
                <select
                  value={String((v() as { value: boolean }).value)}
                  onChange={(e) => setValue({ kind: "bool", value: e.currentTarget.value === "true" })}
                >
                  <option value="true">yes</option>
                  <option value="false">no</option>
                </select>
              </label>
            </Show>
            <Show when={v().kind === "today"}>
              <label>
                Days from today
                <input
                  type="number"
                  step={1}
                  value={(v() as { value: number }).value}
                  onInput={(e) => {
                    const n = Number(e.currentTarget.value);
                    if (e.currentTarget.value.trim() !== "" && Number.isSafeInteger(n)) setValue({ kind: "today", value: n });
                  }}
                />
              </label>
            </Show>
          </>
        )}
      </Show>
      <button type="button" aria-label="Remove condition" onClick={() => props.onChange(null)}>
        Remove
      </button>
    </div>
  );
}
