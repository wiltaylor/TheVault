// Settings > Tools: script tools, which agents call like their built-in tools (script-tools T2).
// Each one is a tool file, `tools/<name>.wcl` (its description, params, script and function, and
// what the script may reach), and the script it runs, `scripts/<script>.ws`, edited together on
// one screen, with a form to try the tool with params. Writing and running need `admin`, so the
// apps leave the page out.

import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createResource, createSignal, on } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ApiError, type Json, type ScriptTool, type ToolParam, type ToolRun, api } from "../api";
import { isName } from "../defs/typeModel";
import { CodeMirror } from "../blocks/LazyCode";
import { errorText } from "./TypeEditor";

export const TOOLS_BASE = "/settings/tools";

/** A new tool's file, naming a script of its own name. */
export function starterWcl(name: string): string {
  return `description = "Say hello to someone."
script      = "${name || "my_tool"}"
function    = "run"

param name {
  kind        = "string"
  description = "Who to greet."
  required    = true
}
`;
}

/** A new tool's script: one function, called with a map of the params. */
export const STARTER_SCRIPT = `// Called with one argument: a map of the tool's params. Return Err("why") to fail the call.
fn run(params: Value) -> Result[Value, string] {
    let who = params.get("name").unwrap().as_string().unwrap()
    Ok(Value::String("Hello, " + who))
}
`;

/** The script a tool file names, read from its `script = "..."` line; empty when it names none. */
export function scriptOf(wcl: string): string {
  const m = /^\s*script\s*=\s*"([^"]*)"/m.exec(wcl);
  return m?.[1] ?? "";
}

/** The value of one param as the run form holds it: text, or a checkbox's state. */
type FormValue = string | boolean;

/**
 * The params the run form sends: each filled param in its kind, an empty optional one left out.
 * Throws with the param's name when a value does not read as its kind.
 */
export function paramValues(params: readonly ToolParam[], form: Record<string, FormValue>): Record<string, Json> {
  const out: Record<string, Json> = {};
  const item = (kind: string, text: string, name: string): Json => {
    switch (kind) {
      case "int": {
        const n = Number(text);
        if (!Number.isInteger(n)) throw new Error(`${name} must be a whole number.`);
        return n;
      }
      case "float": {
        const n = Number(text);
        if (text.trim() === "" || Number.isNaN(n)) throw new Error(`${name} must be a number.`);
        return n;
      }
      case "bool":
        if (text === "true" || text === "false") return text === "true";
        throw new Error(`${name} must be true or false.`);
      case "json":
        try {
          return JSON.parse(text) as Json;
        } catch {
          throw new Error(`${name} must be JSON.`);
        }
      default:
        return text;
    }
  };
  for (const p of params) {
    const v = form[p.name];
    if (p.kind === "bool") {
      if (typeof v === "boolean") out[p.name] = v;
      else if (p.required) out[p.name] = false;
      continue;
    }
    const text = typeof v === "string" ? v : "";
    if (text.trim() === "") {
      if (p.required) throw new Error(`${p.name} is required.`);
      continue;
    }
    if (p.kind === "list") {
      out[p.name] = text
        .split("\n")
        .map((l) => l.trim())
        .filter((l) => l !== "")
        .map((l) => item(p.of ?? "string", l, p.name));
      continue;
    }
    out[p.name] = item(p.kind, text, p.name);
  }
  return out;
}

/** Every script tool: what it does, the script it runs, and whether it checks. */
export function ToolsList() {
  const [tools] = createResource(api.tools);
  return (
    <section class="list-page tools-page">
      <header class="note-header">
        <div>
          <h1>Tools</h1>
          <p class="muted">
            Script tools agents can call. Each runs a wscript function with the params the agent gives it, and makes its
            changes as the agent that called it.
          </p>
        </div>
        <A href={`${TOOLS_BASE}/new`} class="button">
          New tool
        </A>
      </header>
      <Show when={tools.error}>
        <p class="error" role="alert">
          {errorText(tools.error)}
        </p>
      </Show>
      <Show when={tools()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">There are no script tools yet.</p>}>
            <table class="admin-table" aria-label="Script tools">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Description</th>
                  <th>Script</th>
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(t) => (
                    <tr data-tool={t.name} classList={{ failing: !t.valid }}>
                      <td>
                        <A href={`${TOOLS_BASE}/${t.name}`}>{t.name}</A>
                        <Show when={!t.valid}>
                          <span class="badge danger-badge">not offered</span>
                        </Show>
                        <Show when={t.problems.length > 0}>
                          <ul class="error">
                            <For each={t.problems}>{(p) => <li>{p}</li>}</For>
                          </ul>
                        </Show>
                      </td>
                      <td>{t.description ?? ""}</td>
                      <td>
                        <Show when={t.script} fallback={<span class="muted">—</span>}>
                          <code>{t.script}</code>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        )}
      </Show>
    </section>
  );
}

/** What a new tool's save said, shown once its own screen opens. */
const justCreated = new Map<string, string>();

/**
 * Create or edit a script tool on one screen: its file as WCL, the script it names, the problems
 * that keep it from being offered, and a form to run it with params.
 */
export function ToolEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const isNew = () => params.name === undefined;
  const [name, setName] = createSignal("");
  const [wcl, setWcl] = createSignal("");
  const [wclTouched, setWclTouched] = createSignal(false);
  const [source, setSource] = createSignal(STARTER_SCRIPT);
  const [savedSource, setSavedSource] = createSignal<string | null>(null);
  const [tool, setTool] = createSignal<ScriptTool | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [form, setForm] = createStore<Record<string, FormValue>>({});
  const [run, setRun] = createSignal<ToolRun | null>(null);
  const [runError, setRunError] = createSignal<string | null>(null);
  const [running, setRunning] = createSignal(false);

  const script = () => scriptOf(wcl());

  createEffect(
    on(
      () => params.name,
      async (n) => {
        setLoaded(false);
        setLoadError(null);
        setError(null);
        setStatus(null);
        setRun(null);
        setRunError(null);
        setForm(reconcile({}));
        setTool(null);
        if (n === undefined) {
          setName("");
          setWcl(starterWcl(""));
          setWclTouched(false);
          setSource(STARTER_SCRIPT);
          setSavedSource(null);
          setLoaded(true);
          return;
        }
        try {
          const t = await api.tool(n);
          setName(n);
          setTool(t);
          setWcl(t.wcl);
          setWclTouched(true);
          setSource(t.script_source ?? STARTER_SCRIPT);
          setSavedSource(t.script_source);
          setLoaded(true);
          const saved = justCreated.get(n);
          justCreated.delete(n);
          if (saved) setStatus(saved);
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? `There is no tool ${n}.` : errorText(e));
        }
      },
    ),
  );

  const localIssue = () =>
    isNew() && !isName(name()) ? "Give it a name: a-z, 0-9 and _, starting with a letter." : script() ? null : 'Name the script it runs: script = "<name>".';

  async function save() {
    setBusy(true);
    setError(null);
    setStatus(null);
    const n = name();
    try {
      // The script goes with the tool file when it changed, or when the tool's script is new.
      const scriptChanged = source() !== savedSource() || script() !== (tool()?.script ?? null);
      const t = await api.putTool(n, wcl(), scriptChanged ? source() : null);
      const said = t.problems.length > 0 ? "Saved, but agents are not offered it until its problems are fixed." : "Saved.";
      if (isNew()) {
        justCreated.set(n, said);
        navigate(`${TOOLS_BASE}/${n}`);
        return;
      }
      setTool(t);
      setWcl(t.wcl);
      setSavedSource(t.script_source);
      if (t.script_source !== null) setSource(t.script_source);
      setStatus(said);
    } catch (e) {
      setError(e instanceof ApiError && Array.isArray(e.body.issues) ? `Not saved: ${errorText(e)}` : errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete the tool ${name()}? Its script is deleted too unless something else runs it.`)) return;
    setBusy(true);
    try {
      await api.deleteTool(name());
      navigate(TOOLS_BASE);
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  }

  async function runTool(ev: SubmitEvent) {
    ev.preventDefault();
    setRun(null);
    setRunError(null);
    let values: Record<string, Json>;
    try {
      values = paramValues(tool()?.tool?.params ?? [], form);
    } catch (e) {
      setRunError(errorText(e));
      return;
    }
    setRunning(true);
    try {
      setRun(await api.runTool(name(), values));
    } catch (e) {
      setRunError(errorText(e));
    } finally {
      setRunning(false);
    }
  }

  const runParams = () => tool()?.tool?.params ?? [];

  return (
    <section class="def-editor tool-editor">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={loaded()}>
        <header class="note-header">
          <div>
            <p class="note-kind">
              <A href={TOOLS_BASE}>Tools</A>
            </p>
            <h1>{name() || "New tool"}</h1>
          </div>
        </header>

        <Show when={isNew()}>
          <fieldset class="field-card">
            <legend>Name</legend>
            <div class="form-grid">
              <label for="tool-name">Name</label>
              <div>
                <input
                  id="tool-name"
                  value={name()}
                  placeholder="count_tagged"
                  onInput={(e) => {
                    const n = e.currentTarget.value;
                    setName(n);
                    if (!wclTouched()) setWcl(starterWcl(n));
                  }}
                />
                <p class="muted hint">Agents call it by this name. Saved as tools/{name() || "<name>"}.wcl.</p>
              </div>
            </div>
          </fieldset>
        </Show>

        <fieldset class="field-card">
          <legend>Tool file</legend>
          <p class="muted hint">
            Its description (what the agent reads), its <code>param</code> blocks, the <code>script</code> and{" "}
            <code>function</code> it runs, and optionally <code>allow_domains</code>, <code>secrets</code> and{" "}
            <code>timeout</code> in seconds.
          </p>
          <textarea
            class="mono tool-wcl"
            aria-label="Tool file"
            rows={14}
            spellcheck={false}
            value={wcl()}
            onInput={(e) => {
              setWcl(e.currentTarget.value);
              setWclTouched(true);
              setStatus(null);
            }}
          />
        </fieldset>

        <fieldset class="field-card">
          <legend>Script</legend>
          <p class="muted hint">
            wscript, saved as scripts/{script() || "<script>"}.ws. Its function gets one argument, a map of the params, and
            its result goes back to the agent. The <code>vault</code>, <code>http</code>, <code>ai</code> and{" "}
            <code>secrets</code> modules are available.
          </p>
          <div class="script-editor scheduled-script">
            <CodeMirror
              source={source()}
              language="rust"
              onChange={(s) => {
                setSource(s);
                setStatus(null);
              }}
            />
          </div>
        </fieldset>

        <Show when={(tool()?.problems.length ?? 0) > 0}>
          <ul class="error" role="alert" aria-label="Tool problems">
            <For each={tool()?.problems}>{(p) => <li>{p}</li>}</For>
          </ul>
        </Show>
        <Show when={localIssue()}>
          <p class="muted">{localIssue()}</p>
        </Show>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <div class="actions">
          <button type="button" class="primary" disabled={busy() || localIssue() !== null} onClick={() => void save()}>
            Save
          </button>
          <Show when={!isNew()}>
            <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
              Delete
            </button>
          </Show>
          <Show when={status()}>
            <span class="muted" role="status">
              {status()}
            </span>
          </Show>
        </div>

        <Show when={!isNew() && tool()?.valid}>
          <form class="field-card tool-run" aria-label="Run the tool" onSubmit={(e) => void runTool(e)}>
            <h2>Run</h2>
            <p class="muted hint">Try the saved tool with params. Its changes are made as you.</p>
            <div class="form-grid">
              <For each={runParams()}>
                {(p) => (
                  <>
                    <label for={`tool-param-${p.name}`}>
                      {p.name}
                      <Show when={p.required}> *</Show>
                    </label>
                    <div>
                      <Switch
                        fallback={
                          <input
                            id={`tool-param-${p.name}`}
                            type={p.kind === "int" || p.kind === "float" ? "number" : "text"}
                            step={p.kind === "float" ? "any" : undefined}
                            value={(form[p.name] as string | undefined) ?? ""}
                            onInput={(e) => setForm(p.name, e.currentTarget.value)}
                          />
                        }
                      >
                        <Match when={p.kind === "bool"}>
                          <input
                            id={`tool-param-${p.name}`}
                            type="checkbox"
                            checked={form[p.name] === true}
                            onChange={(e) => setForm(p.name, e.currentTarget.checked)}
                          />
                        </Match>
                        <Match when={p.kind === "json" || p.kind === "list"}>
                          <textarea
                            id={`tool-param-${p.name}`}
                            class="mono"
                            rows={3}
                            value={(form[p.name] as string | undefined) ?? ""}
                            onInput={(e) => setForm(p.name, e.currentTarget.value)}
                          />
                        </Match>
                      </Switch>
                      <p class="muted hint">
                        {p.kind === "list" ? `A list of ${p.of ?? "string"}, one per line. ` : p.kind === "json" ? "JSON. " : ""}
                        {p.description ?? ""}
                      </p>
                    </div>
                  </>
                )}
              </For>
            </div>
            <div class="actions">
              <button type="submit" disabled={running()}>
                {running() ? "Running…" : "Run"}
              </button>
            </div>
            <Show when={runError()}>
              <p class="error" role="alert">
                {runError()}
              </p>
            </Show>
            <Show when={run()}>
              {(r) => (
                <div class="tool-run-result" data-testid="tool-run-result">
                  <p role="status" classList={{ error: !r().ok }}>
                    {r().ok ? "Succeeded" : `Failed${r().kind ? ` (${r().kind})` : ""}: ${r().error ?? ""}`}
                    <span class="muted">
                      {r().duration_ms !== undefined ? ` · ${r().duration_ms} ms` : ""}
                      {r().instructions !== undefined ? ` · ${r().instructions} instructions` : ""}
                    </span>
                  </p>
                  <Show when={r().ok}>
                    <pre class="mono" aria-label="Result">
                      {JSON.stringify(r().result ?? null, null, 2)}
                    </pre>
                  </Show>
                  <Show when={r().log}>
                    <pre class="mono" aria-label="Log">
                      {r().log}
                    </pre>
                  </Show>
                </div>
              )}
            </Show>
          </form>
        </Show>
      </Show>
    </section>
  );
}
