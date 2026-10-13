import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createResource, createSignal, on } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ApiError, type Task, type TaskRun, api } from "../api";
import { when } from "../admin/format";
import { type TaskModel, emptyTask, lines, taskFromApi, taskProblems, taskWcl } from "../admin/taskModel";
import { isName } from "../defs/typeModel";
import { errorText } from "./TypeEditor";

/** A run's outcome as a badge: failed runs stand out. */
function Outcome(props: { outcome: TaskRun["outcome"] }) {
  return (
    <span class={`outcome outcome-${props.outcome}`} data-outcome={props.outcome}>
      {{ running: "Running", succeeded: "Succeeded", failed: "Failed" }[props.outcome]}
    </span>
  );
}

/** Every scheduled task, when it runs next, and whether it is failing (D10). */
export function TaskList() {
  const [tasks] = createResource(api.tasks);
  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Tasks</h1>
        <A href="/settings/tasks/new" class="button">
          New task
        </A>
      </header>
      <Show when={tasks.error}>
        <p class="error" role="alert">
          {errorText(tasks.error)}
        </p>
      </Show>
      <Show when={tasks()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No scheduled tasks yet.</p>}>
            <table class="admin-table" aria-label="Tasks">
              <thead>
                <tr>
                  <th>Task</th>
                  <th>Schedule</th>
                  <th>Next run</th>
                  <th>Latest run</th>
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(t) => (
                    <tr data-task={t.name} classList={{ failing: t.failing }}>
                      <td>
                        <A href={`/settings/tasks/${t.name}`}>{t.label ?? t.name}</A>
                        <Show when={t.paused}>
                          <span class="badge muted-badge">paused</span>
                        </Show>
                        <Show when={t.failing}>
                          <span class="badge danger-badge">failing</span>
                        </Show>
                        <Show when={t.error}>
                          <span class="error"> {t.error}</span>
                        </Show>
                      </td>
                      <td>
                        <code>{t.schedule ?? "—"}</code> <span class="muted">{t.timezone}</span>
                      </td>
                      <td>{t.paused ? "—" : when(t.next_run)}</td>
                      <td>
                        <Show when={t.latest_run} fallback={<span class="muted">Never run</span>}>
                          {(r) => (
                            <>
                              <Outcome outcome={r().outcome} /> <span class="muted">{when(r().started)}</span>
                            </>
                          )}
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

/** One run in the history, with its log shown on request. */
function RunItem(props: { run: TaskRun; open: boolean }) {
  return (
    <li data-run={props.run.id} classList={{ failing: props.run.outcome === "failed" }}>
      <details open={props.open}>
        <summary>
          <Outcome outcome={props.run.outcome} /> {when(props.run.started)}
          <span class="muted">
            {" "}
            · attempt {props.run.attempt}
            {props.run.ended !== null ? ` · ${((props.run.ended - props.run.started) / 1000).toFixed(1)} s` : ""}
          </span>
        </summary>
        <Show when={props.run.message}>
          <p class="error">{props.run.message}</p>
        </Show>
        <pre class="run-log" aria-label={`Log of run ${props.run.id}`}>
          {props.run.log || "(no output)"}
        </pre>
      </details>
    </li>
  );
}

/**
 * The task editor (D10): the task file's items, a button to run it now, and its run history with
 * each run's log. Secret values are set on the secrets page; a task only names the ones it reads.
 */
export function TaskEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const isNew = () => params.name === undefined;
  const [name, setName] = createSignal("");
  const [model, setModel] = createStore<TaskModel>(emptyTask());
  const [task, setTask] = createSignal<Task | null>(null);
  const [runs, setRuns] = createSignal<TaskRun[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The run "Run now" started, whose log opens in the history.
  const [justRan, setJustRan] = createSignal<number | null>(null);
  const [scripts] = createResource(() => api.scripts().catch(() => [] as string[]), { initialValue: [] });
  const [secrets] = createResource(
    () => api.secrets().then((s) => s.map((x) => x.name)).catch(() => [] as string[]),
    { initialValue: [] },
  );

  async function load(name: string) {
    const [t, r] = await Promise.all([api.task(name), api.taskRuns(name)]);
    setTask(t);
    setRuns(r);
    return t;
  }

  createEffect(
    on(
      () => params.name,
      async (n) => {
        setLoaded(false);
        setLoadError(null);
        setError(null);
        setStatus(null);
        setJustRan(null);
        setTask(null);
        setRuns([]);
        if (n === undefined) {
          setName("");
          setModel(reconcile(emptyTask()));
          setLoaded(true);
          return;
        }
        try {
          const t = await load(n);
          setName(n);
          setModel(reconcile(taskFromApi(t)));
          setLoaded(true);
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? `There is no task ${n}.` : errorText(e));
        }
      },
    ),
  );

  const issues = () => [
    ...(isNew() && !isName(name()) ? ["The task needs a name: a-z, 0-9 and _."] : []),
    ...taskProblems(model),
  ];

  async function save() {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const t = await api.putTask(name(), taskWcl(model));
      if (isNew()) {
        navigate(`/settings/tasks/${name()}`);
        return;
      }
      setTask(t);
      setStatus("Saved.");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function runNow() {
    setBusy(true);
    setError(null);
    setStatus("Running…");
    try {
      const run = await api.runTask(name());
      setJustRan(run.id);
      await load(name());
      setStatus(run.outcome === "failed" ? `The run failed: ${run.message ?? "see its log"}` : "The run succeeded.");
    } catch (e) {
      setStatus(null);
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete task ${name()}? Its run history stays.`)) return;
    setBusy(true);
    try {
      await api.deleteTask(name());
      navigate("/settings/tasks");
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  }

  type TextKey = "label" | "description" | "schedule" | "timezone" | "function" | "retry_delay" | "timeout";
  const Text = (props: { id: string; item: TextKey; placeholder?: string }) => (
    <input
      id={props.id}
      placeholder={props.placeholder}
      value={model[props.item]}
      onInput={(e) => {
        setModel(props.item, e.currentTarget.value);
        setStatus(null);
      }}
    />
  );

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
              <A href="/settings/tasks">Tasks</A>
            </p>
            <h1>
              {model.label || name() || "New task"}
              <Show when={task()?.failing}>
                <span class="badge danger-badge">failing</span>
              </Show>
            </h1>
          </div>
          <Show when={!isNew()}>
            <div class="actions">
              <button type="button" class="primary" disabled={busy()} onClick={() => void runNow()}>
                Run now
              </button>
              <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
                Delete task
              </button>
            </div>
          </Show>
        </header>
        <Show when={task()?.error}>
          <p class="error" role="alert">
            The task file does not check: {task()?.error}
          </p>
        </Show>
        <Show when={task() && !task()!.error}>
          <p class="muted">
            {model.paused ? "Paused: it runs only when run by hand." : `Next run: ${when(task()!.next_run)}.`}
            <Show when={task()!.retry}>
              {(r) => ` Retry ${r().attempt} at ${when(r().at)}.`}
            </Show>
          </p>
        </Show>
        <div class="form-grid">
          <Show when={isNew()}>
            <label for="task-name">Name</label>
            <input id="task-name" value={name()} placeholder="digest" onInput={(e) => setName(e.currentTarget.value)} />
          </Show>
          <label for="task-label">Label</label>
          <Text id="task-label" item="label" />
          <label for="task-description">Description</label>
          <Text id="task-description" item="description" />
          <label for="task-schedule">Schedule (cron)</label>
          <Text id="task-schedule" item="schedule" placeholder="0 7 * * 1-5" />
          <label for="task-timezone">Time zone</label>
          <Text id="task-timezone" item="timezone" placeholder="UTC" />
          <label for="task-script">Script</label>
          <select id="task-script" onChange={(e) => setModel("script", e.currentTarget.value)}>
            <option value="" selected={!model.script}>
              —
            </option>
            <For each={[...new Set([...scripts(), ...(model.script ? [model.script] : [])])].sort()}>
              {(s) => (
                <option value={s} selected={model.script === s}>
                  {s}
                </option>
              )}
            </For>
          </select>
          <label for="task-function">Function</label>
          <Text id="task-function" item="function" placeholder="run" />
          <label for="task-domains">Allowed hosts, one per line</label>
          <textarea
            id="task-domains"
            rows={2}
            value={model.allow_domains.join("\n")}
            onChange={(e) => setModel("allow_domains", lines(e.currentTarget.value))}
          />
          <label for="task-secrets">Secrets, one per line</label>
          <div>
            <textarea
              id="task-secrets"
              rows={2}
              value={model.secrets.join("\n")}
              onChange={(e) => setModel("secrets", lines(e.currentTarget.value))}
            />
            <p class="muted hint">
              Known secrets: {secrets().length ? secrets().join(", ") : "none"}. <A href="/settings/secrets">Set values</A>
            </p>
          </div>
          <label for="task-retries">Retries</label>
          <input
            id="task-retries"
            type="number"
            min="0"
            step="1"
            value={model.retries}
            onInput={(e) => setModel("retries", e.currentTarget.valueAsNumber)}
          />
          <label for="task-retry-delay">First retry after</label>
          <Text id="task-retry-delay" item="retry_delay" placeholder="PT1M" />
          <label for="task-timeout">Time limit</label>
          <Text id="task-timeout" item="timeout" placeholder="PT30S" />
          <span />
          <label>
            <input type="checkbox" checked={model.paused} onChange={(e) => setModel("paused", e.currentTarget.checked)} />{" "}
            Paused
          </label>
        </div>
        <div class="save-bar">
          <Show when={issues().length > 0}>
            <ul class="error" role="alert" aria-label="Problems">
              <For each={issues()}>{(p) => <li>{p}</li>}</For>
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
          <button type="button" class="primary" disabled={busy() || issues().length > 0} onClick={() => void save()}>
            {isNew() ? "Create task" : "Save task"}
          </button>
        </div>
        <Show when={!isNew()}>
          <h2>Runs</h2>
          <Show when={runs().length > 0} fallback={<p class="muted">No runs yet.</p>}>
            <ol class="run-list" aria-label="Runs">
              <For each={runs()}>{(r) => <RunItem run={r} open={r.id === justRan()} />}</For>
            </ol>
          </Show>
        </Show>
      </Show>
    </section>
  );
}
