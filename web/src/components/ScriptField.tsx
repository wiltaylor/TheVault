// Scripts are chosen and edited where they are used: a type's handlers and migration, and a
// scheduled task. There is no separate scripts page.
import { For, Show, createEffect, createResource, createSignal, on, onMount } from "solid-js";
import { ApiError, type ScriptProblem, api } from "../api";
import { CodeMirror } from "../blocks/CodeBlock";
import { isName } from "../defs/typeModel";

const ROLE_LABELS: Record<ScriptProblem["role"], string> = {
  before_handler: "As a before handler",
  after_handler: "As an after handler",
  task: "As a task",
};

const STARTER = `fn run() {
}
`;

// Not a valid script name, so it can never collide with one.
const NEW = "__new__";

/**
 * Every script name, shared by the script fields on the page. Loaded when a field first appears,
 * and again whenever another one does, so scripts added elsewhere, by an agent say, show up.
 */
const [wanted, setWanted] = createSignal(0);
const [scriptNames, { refetch: refetchScripts }] = createResource(
  () => wanted() > 0 && wanted(),
  () => api.scripts().catch(() => [] as string[]),
  { initialValue: [] as string[] },
);

export { scriptNames };

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Choose a script by name, start a new one, or edit the chosen one in place. `value` is the script
 * name, empty for none.
 */
export function ScriptField(props: {
  id: string;
  value: string;
  onChange: (name: string) => void;
  noneLabel?: string;
}) {
  const [editing, setEditing] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  const [createError, setCreateError] = createSignal<string | null>(null);

  onMount(() => setWanted((n) => n + 1));

  const names = () => [...new Set([...scriptNames(), ...(props.value ? [props.value] : [])])].sort();

  async function create(ev: SubmitEvent) {
    ev.preventDefault();
    const n = newName().trim();
    setCreateError(null);
    if (!isName(n)) {
      setCreateError("A script name uses a-z, 0-9 and _, and starts with a letter.");
      return;
    }
    try {
      if (!scriptNames().includes(n)) await api.putScript(n, STARTER);
      await refetchScripts();
      props.onChange(n);
      setCreating(false);
      setNewName("");
      setEditing(true);
    } catch (e) {
      setCreateError(message(e));
    }
  }

  return (
    <div class="script-field">
      <div class="script-field-row">
        <select
          id={props.id}
          onChange={(e) => {
            const v = e.currentTarget.value;
            if (v === NEW) {
              setCreating(true);
              e.currentTarget.value = props.value;
              return;
            }
            props.onChange(v);
            if (!v) setEditing(false);
          }}
        >
          <option value="" selected={!props.value}>
            {props.noneLabel ?? "—"}
          </option>
          <For each={names()}>
            {(n) => (
              <option value={n} selected={props.value === n}>
                {n}
              </option>
            )}
          </For>
          <option value={NEW}>New script…</option>
        </select>
        <Show when={props.value}>
          <button type="button" aria-expanded={editing()} onClick={() => setEditing(!editing())}>
            {editing() ? "Close script" : "Edit script"}
          </button>
        </Show>
      </div>
      <Show when={creating()}>
        <form class="inline-form" aria-label="New script" onSubmit={create}>
          <input
            aria-label="New script name"
            placeholder="digest"
            value={newName()}
            onInput={(e) => setNewName(e.currentTarget.value)}
          />
          <button type="submit">Create script</button>
          <button type="button" onClick={() => setCreating(false)}>
            Cancel
          </button>
        </form>
        <Show when={createError()}>
          <p class="error" role="alert">
            {createError()}
          </p>
        </Show>
      </Show>
      <Show when={editing() && props.value} keyed>
        {(name) => (
          <ScriptPanel
            name={name}
            onDeleted={() => {
              setEditing(false);
              props.onChange("");
              void refetchScripts();
            }}
          />
        )}
      </Show>
    </div>
  );
}

/** One script in CodeMirror. Saving stores it as written and shows why it would not compile. */
function ScriptPanel(props: { name: string; onDeleted: () => void }) {
  const [source, setSource] = createSignal("");
  const [loaded, setLoaded] = createSignal(false);
  const [problems, setProblems] = createSignal<ScriptProblem[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  createEffect(
    on(
      () => props.name,
      async (name) => {
        setLoaded(false);
        setError(null);
        setStatus(null);
        try {
          const s = await api.script(name);
          setSource(s.source);
          setProblems(s.problems);
          setLoaded(true);
        } catch (e) {
          setError(e instanceof ApiError && e.status === 404 ? `There is no script ${name}.` : message(e));
        }
      },
    ),
  );

  async function save() {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const s = await api.putScript(props.name, source());
      setProblems(s.problems);
      setStatus(s.problems.length ? "Saved, but the script does not compile." : "Saved.");
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete script ${props.name}?`)) return;
    setBusy(true);
    setError(null);
    try {
      await api.deleteScript(props.name);
      props.onDeleted();
    } catch (e) {
      const by = e instanceof ApiError ? (e.body.by as string[] | undefined) : undefined;
      setError(by?.length ? `${message(e)}: it is used by ${by.join(", ")}.` : message(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="script-panel" role="group" aria-label={`Script ${props.name}`}>
      <Show when={loaded()} fallback={<Show when={!error()}><p class="muted">Loading script…</p></Show>}>
        <div class="script-editor">
          <CodeMirror
            source={source()}
            language="rust"
            onChange={(s) => {
              setSource(s);
              setStatus(null);
            }}
          />
        </div>
      </Show>
      <Show when={problems().length > 0}>
        <ul class="error" role="alert" aria-label="Script problems">
          <For each={problems()}>
            {(p) => (
              <li>
                <strong>{ROLE_LABELS[p.role]}:</strong> {p.message}
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="script-panel-actions">
        <Show when={status()}>
          <p role="status">{status()}</p>
        </Show>
        <button type="button" class="danger" disabled={busy() || !loaded()} onClick={() => void remove()}>
          Delete script
        </button>
        <button type="button" class="primary" disabled={busy() || !loaded()} onClick={() => void save()}>
          Save script
        </button>
      </div>
    </div>
  );
}
