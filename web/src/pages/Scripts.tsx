import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createResource, createSignal, on } from "solid-js";
import { ApiError, type ScriptProblem, api } from "../api";
import { CodeMirror } from "../blocks/CodeBlock";
import { isName } from "../defs/typeModel";
import { errorText } from "./TypeEditor";

const ROLE_LABELS: Record<ScriptProblem["role"], string> = {
  before_handler: "As a before handler",
  after_handler: "As an after handler",
  task: "As a task",
};

const STARTER = `fn run() {
}
`;

/** Every script in `scripts/`, and a form to start a new one (D9, D10). */
export function ScriptList() {
  const navigate = useNavigate();
  const [scripts, { refetch }] = createResource(api.scripts);
  const [name, setName] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);

  async function create(ev: SubmitEvent) {
    ev.preventDefault();
    const n = name().trim();
    setError(null);
    if (!isName(n)) {
      setError("A script name uses a-z, 0-9 and _, and starts with a letter.");
      return;
    }
    if (scripts()?.includes(n)) {
      navigate(`/settings/scripts/${n}`);
      return;
    }
    try {
      await api.putScript(n, STARTER);
      await refetch();
      navigate(`/settings/scripts/${n}`);
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Scripts</h1>
      </header>
      <p class="muted">Handlers, migrations and scheduled tasks run functions of these wscript files.</p>
      <form class="inline-form" onSubmit={create} aria-label="Create a script">
        <label for="new-script">New script</label>
        <input id="new-script" value={name()} placeholder="digest" onInput={(e) => setName(e.currentTarget.value)} />
        <button type="submit">Create script</button>
      </form>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={scripts()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No scripts yet.</p>}>
            <ul class="admin-list" aria-label="Scripts">
              <For each={list()}>
                {(s) => (
                  <li>
                    <A href={`/settings/scripts/${s}`}>{s}</A>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        )}
      </Show>
    </section>
  );
}

/** One script in CodeMirror. Saving stores it as written and shows why it would not compile. */
export function ScriptEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const [source, setSource] = createSignal("");
  const [loaded, setLoaded] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [problems, setProblems] = createSignal<ScriptProblem[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  createEffect(
    on(
      () => params.name,
      async (name) => {
        setLoaded(false);
        setLoadError(null);
        setError(null);
        setStatus(null);
        try {
          const s = await api.script(name ?? "");
          setSource(s.source);
          setProblems(s.problems);
          setLoaded(true);
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? `There is no script ${name}.` : errorText(e));
        }
      },
    ),
  );

  async function save() {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const s = await api.putScript(params.name!, source());
      setProblems(s.problems);
      setStatus(s.problems.length ? "Saved, but the script does not compile." : "Saved.");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete script ${params.name}?`)) return;
    setBusy(true);
    setError(null);
    try {
      await api.deleteScript(params.name!);
      navigate("/settings/scripts");
    } catch (e) {
      const by = e instanceof ApiError ? (e.body.by as string[] | undefined) : undefined;
      setError(by?.length ? `${errorText(e)}: it is used by ${by.join(", ")}.` : errorText(e));
    } finally {
      setBusy(false);
    }
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
          <div>
            <p class="note-kind">
              <A href="/settings/scripts">Scripts</A>
            </p>
            <h1>{params.name}</h1>
          </div>
          <div class="actions">
            <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
              Delete script
            </button>
          </div>
        </header>
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
        <div class="save-bar">
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
          <Show when={status()}>
            <p role="status">{status()}</p>
          </Show>
          <button type="button" class="primary" disabled={busy()} onClick={() => void save()}>
            Save script
          </button>
        </div>
      </Show>
    </section>
  );
}
