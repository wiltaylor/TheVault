// Settings > Scheduled: scripts that run on a cron schedule (D10). Each one is a task file,
// `tasks/<name>.wcl`, and the script it runs, `scripts/<script>.ws`, edited together on one screen.

import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Index, Show, createEffect, createResource, createSignal, on, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { ApiError, type CronPreview, type ScriptProblem, type Task, type TaskRun, api, waitForRun } from "../api";
import { when } from "../admin/format";
import {
  type DirAccess,
  type Preset,
  type TaskModel,
  type WebAccess,
  cronOf,
  dirAccess,
  emptyTask,
  lines,
  presetOf,
  splitDirs,
  taskFromApi,
  taskProblems,
  taskWcl,
  webAccess,
} from "../admin/taskModel";
import { isName } from "../defs/typeModel";
import { CodeMirror } from "../blocks/LazyCode";
import { errorText } from "./TypeEditor";

const BASE = "/settings/scheduled";

const STARTER = `use vault

// Runs on the schedule above. Return Err("why") to mark the run as failed.
fn run() -> Result[string, string] {
    vault::log("Hello from a scheduled script")
    Ok("done")
}
`;

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/** The browser's IANA time zone, the default for a new scheduled script. */
function browserZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** Every IANA time zone the browser knows, for the time zone picker. */
export function zones(): string[] {
  const all =
    (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  return all.includes("UTC") ? all : ["UTC", ...all];
}

/** A time in milliseconds as the task's time zone reads it. */
export function inZone(ms: number, zone: string): string {
  const opts: Intl.DateTimeFormatOptions = {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  };
  try {
    return new Intl.DateTimeFormat(undefined, { ...opts, timeZone: zone }).format(ms);
  } catch {
    return new Intl.DateTimeFormat(undefined, opts).format(ms);
  }
}

function duration(run: TaskRun): string {
  return run.ended === null ? "" : `${((run.ended - run.started) / 1000).toFixed(1)} s`;
}

/** A run's outcome as a badge: failed runs stand out. */
export function Outcome(props: { outcome: TaskRun["outcome"] }) {
  return (
    <span class={`outcome outcome-${props.outcome}`} data-outcome={props.outcome}>
      {{ running: "Running", succeeded: "Succeeded", failed: "Failed" }[props.outcome]}
    </span>
  );
}

/** Every scheduled script: when it runs, in words, when it runs next, and how it last went. */
export function ScheduledList() {
  const [tasks, { refetch }] = createResource(api.tasks);
  const [running, setRunning] = createSignal<string | null>(null);
  const [results, setResults] = createStore<Record<string, string>>({});
  let disposed = false;
  onCleanup(() => (disposed = true));

  // Run now answers once the run starts; the page reads the run until it ends (ticket 115).
  async function runNow(name: string) {
    setRunning(name);
    setResults(name, "Running…");
    try {
      const run = await waitForRun(await api.runTask(name), (id) => api.taskRun(name, id), undefined, () => disposed);
      if (run.outcome === "running") return;
      setResults(name, run.outcome === "failed" ? `Failed: ${run.message ?? "see its log"}` : "Succeeded");
      await refetch();
    } catch (e) {
      setResults(name, errorText(e));
    } finally {
      setRunning(null);
    }
  }

  return (
    <section class="list-page scheduled-page">
      <header class="note-header">
        <div>
          <h1>Scheduled</h1>
          <p class="muted">
            Scripts that run on a schedule. They can call web services, read and write files in the directories you
            allow, and change notes.
          </p>
        </div>
        <A href={`${BASE}/new`} class="button">
          New scheduled script
        </A>
      </header>
      <Show when={tasks.error}>
        <p class="error" role="alert">
          {errorText(tasks.error)}
        </p>
      </Show>
      <Show when={tasks()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">Nothing is scheduled yet.</p>}>
            <table class="admin-table" aria-label="Scheduled scripts">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Schedule</th>
                  <th>Next run</th>
                  <th>Last run</th>
                  <th>
                    <span class="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(t) => (
                    <tr data-task={t.name} classList={{ failing: t.failing }}>
                      <td>
                        <A href={`${BASE}/${t.name}`}>{t.label ?? t.name}</A>
                        <Show when={t.paused}>
                          <span class="badge muted-badge">paused</span>
                        </Show>
                        <Show when={t.failing}>
                          <span class="badge danger-badge">failing</span>
                        </Show>
                        <Show when={t.error}>
                          <p class="error">{t.error}</p>
                        </Show>
                      </td>
                      <td>
                        <span data-testid="schedule-text">{t.schedule_text ?? "—"}</span>
                        <Show when={t.schedule}>
                          <br />
                          <code class="muted">{t.schedule}</code>
                        </Show>
                      </td>
                      <td>{t.paused ? <span class="muted">Paused</span> : when(t.next_run)}</td>
                      <td>
                        <Show when={t.latest_run} fallback={<span class="muted">Never run</span>}>
                          {(r) => (
                            <>
                              <Outcome outcome={r().outcome} /> <span class="muted">{when(r().started)}</span>
                              <Show when={duration(r())}>
                                <span class="muted"> · {duration(r())}</span>
                              </Show>
                            </>
                          )}
                        </Show>
                      </td>
                      <td class="scheduled-actions">
                        <button
                          type="button"
                          disabled={running() !== null || !!t.error}
                          onClick={() => void runNow(t.name)}
                        >
                          Run now
                        </button>
                        <Show when={results[t.name]}>
                          <p class="muted hint" role="status">
                            {results[t.name]}
                          </p>
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
            {duration(props.run) ? ` · ${duration(props.run)}` : ""}
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

type Issue = { at: string; message: string };

/** The messages of the server's issues about `at`. */
function FieldIssues(props: { issues: Issue[]; at: string; id?: string }) {
  const mine = () => props.issues.filter((i) => i.at === props.at || i.at.startsWith(`${props.at}.`));
  return (
    <Show when={mine().length > 0}>
      <p class="error field-issue" id={props.id} role="alert" data-issue={props.at}>
        {mine()
          .map((i) => i.message)
          .join("; ")}
      </p>
    </Show>
  );
}

/**
 * What the save of a task just created said, by task name, for the task's own page to show once
 * it opens. The workspace's pane history carries no navigation state, so it waits here.
 */
const justCreated = new Map<string, string>();

/**
 * Create or edit a scheduled script on one screen: its name, its schedule with the next runs
 * previewed by the server's own cron parser, the script's source, what it may reach, and its run
 * history. Secret values are set on the secrets page; a script only names the ones it reads.
 */
export function ScheduledEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const isNew = () => params.name === undefined;
  const [name, setName] = createSignal("");
  const [model, setModel] = createStore<TaskModel>(emptyTask());
  const [dirs, setDirs] = createStore<DirAccess[]>([]);
  const [web, setWeb] = createSignal<WebAccess>("none");
  const [hosts, setHosts] = createSignal("");
  const [source, setSource] = createSignal(STARTER);
  const [savedSource, setSavedSource] = createSignal<string | null>(null);
  const [problems, setProblems] = createSignal<ScriptProblem[]>([]);
  const [task, setTask] = createSignal<Task | null>(null);
  const [runs, setRuns] = createSignal<TaskRun[]>([]);
  // In an app, the runs are the home server's: why they could not be listed, when it cannot be
  // reached or the app is not paired.
  const [runsUnavailable, setRunsUnavailable] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [issues, setIssues] = createSignal<Issue[]>([]);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The run "Run now" started, whose log opens in the history.
  const [justRan, setJustRan] = createSignal<number | null>(null);
  const [preview, setPreview] = createSignal<CronPreview | null>(null);
  const [previewError, setPreviewError] = createSignal<string | null>(null);
  const [secretNames] = createResource(
    () => api.secrets().then((s) => s.map((x) => x.name)).catch(() => [] as string[]),
    { initialValue: [] },
  );
  const zoneList = zones();

  async function load(n: string) {
    const [t, r] = await Promise.all([
      api.task(n),
      api.taskRuns(n).catch((e: unknown) => {
        if (e instanceof ApiError && e.needsServer) return e;
        throw e;
      }),
    ]);
    setTask(t);
    setRunsUnavailable(r instanceof ApiError ? r.message : null);
    setRuns(r instanceof ApiError ? [] : r);
    return t;
  }

  function fill(m: TaskModel) {
    setModel(reconcile(m));
    setDirs(reconcile(dirAccess(m)));
    setWeb(webAccess(m.allow_domains));
    setHosts(m.allow_domains.filter((d) => d !== "*").join("\n"));
  }

  createEffect(
    on(
      () => params.name,
      async (n) => {
        setLoaded(false);
        setLoadError(null);
        setError(null);
        setIssues([]);
        setStatus(null);
        setJustRan(null);
        setTask(null);
        setRuns([]);
        setProblems([]);
        if (n === undefined) {
          setName("");
          fill({ ...emptyTask(), timezone: browserZone(), schedule: "0 7 * * *" });
          setSource(STARTER);
          setSavedSource(null);
          setLoaded(true);
          return;
        }
        try {
          const t = await load(n);
          setName(n);
          fill(taskFromApi(t));
          const script = t.script ?? n;
          try {
            const s = await api.script(script);
            setSource(s.source);
            setSavedSource(s.source);
            setProblems(s.problems);
          } catch {
            setSource(STARTER);
            setSavedSource(null);
          }
          setLoaded(true);
          // A task just created says how its save went, as an edit's save does, once.
          const saved = justCreated.get(n);
          justCreated.delete(n);
          if (saved) setStatus(saved);
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? `Nothing is scheduled as ${n}.` : errorText(e));
        }
      },
    ),
  );

  // The schedule in words and its next runs, from the server, a moment after typing stops.
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  let previewSeq = 0;
  createEffect(
    on([() => model.schedule, () => model.timezone], ([schedule, zone]) => {
      clearTimeout(previewTimer);
      const seq = ++previewSeq;
      if (!schedule.trim()) {
        setPreview(null);
        setPreviewError("Enter a schedule.");
        return;
      }
      previewTimer = setTimeout(async () => {
        try {
          const p = await api.cron(schedule.trim(), zone.trim() || "UTC");
          if (seq !== previewSeq) return;
          setPreview(p);
          setPreviewError(null);
        } catch (e) {
          if (seq !== previewSeq) return;
          setPreview(null);
          setPreviewError(errorText(e).replace(/^(schedule|timezone): /, ""));
        }
      }, 200);
    }),
  );
  onCleanup(() => clearTimeout(previewTimer));
  let disposed = false;
  onCleanup(() => (disposed = true));

  const choice = () => presetOf(model.schedule);
  function choose(preset: Preset, time = choice().time, day = choice().day) {
    setModel("schedule", cronOf({ preset, time, day }, model.schedule));
    setStatus(null);
  }

  const scriptName = () => model.script || name();

  /** The model as it will be saved, with the directory and web access rows folded in. */
  const saving = (): TaskModel => ({
    ...model,
    script: scriptName(),
    allow_domains: web() === "any" ? ["*"] : web() === "hosts" ? lines(hosts()) : [],
    ...splitDirs(dirs),
  });

  const localIssues = () => [
    ...(isNew() && !isName(name()) ? ["Give it a name: a-z, 0-9 and _, starting with a letter."] : []),
    ...taskProblems(saving()).filter((p) => !p.startsWith("Choose the script")),
    ...(web() === "hosts" && lines(hosts()).length === 0 ? ["List the hosts it may reach, or choose another web access."] : []),
  ];

  const taskProblemsOfScript = () => problems().filter((p) => p.role === "task");

  async function save() {
    setBusy(true);
    setError(null);
    setIssues([]);
    setStatus(null);
    const n = name();
    const m = saving();
    try {
      if (isNew() && (await api.scripts()).includes(m.script)) {
        setError(`A script named ${m.script} exists already. Choose another name.`);
        return;
      }
      if (source() !== savedSource()) {
        const s = await api.putScript(m.script, source());
        setSavedSource(source());
        setProblems(s.problems);
      }
      const t = await api.putTask(n, taskWcl(m));
      // Checked again now the task names the script. Before, a script nothing named yet, or
      // only a device file named, was not checked as a task, so a script using the device
      // module, which no task can load, showed as saved (ticket 92).
      const checked = (await api.script(m.script)).problems;
      const saved = checked.some((p) => p.role === "task") ? "Saved, but the script does not compile." : "Saved.";
      if (isNew()) {
        justCreated.set(n, saved);
        navigate(`${BASE}/${n}`);
        return;
      }
      setProblems(checked);
      setTask(t);
      setStatus(saved);
    } catch (e) {
      if (e instanceof ApiError && Array.isArray(e.body.issues)) {
        setIssues(e.body.issues as Issue[]);
        setError("Not saved: fix the problems marked above.");
      } else {
        setError(errorText(e));
      }
    } finally {
      setBusy(false);
    }
  }

  // Run now answers once the run starts; the page reads the run until it ends (ticket 115).
  async function runNow() {
    setBusy(true);
    setError(null);
    setStatus("Running…");
    try {
      const run = await waitForRun(await api.runTask(name()), (id) => api.taskRun(name(), id), undefined, () => disposed);
      if (run.outcome === "running") return;
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
    if (!confirm(`Delete the scheduled script ${name()}? Its run history stays, and so does its script file.`)) return;
    setBusy(true);
    try {
      await api.deleteTask(name());
      navigate(BASE);
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  }

  type TextKey = "label" | "description" | "function" | "retry_delay" | "timeout";
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

  const secretChoices = () => [...new Set([...secretNames(), ...model.secrets])].sort();

  return (
    <section class="def-editor scheduled-editor">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={loaded()}>
        <header class="note-header">
          <div>
            <p class="note-kind">
              <A href={BASE}>Scheduled</A>
            </p>
            <h1>
              {model.label || name() || "New scheduled script"}
              <Show when={task()?.failing}>
                <span class="badge danger-badge">failing</span>
              </Show>
              <Show when={task() && model.paused}>
                <span class="badge muted-badge">paused</span>
              </Show>
            </h1>
          </div>
          <Show when={!isNew()}>
            <div class="actions">
              <button type="button" class="primary" disabled={busy()} onClick={() => void runNow()}>
                Run now
              </button>
              <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
                Delete
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
            {task()!.paused ? "Paused: it runs only when run by hand." : `Next run: ${when(task()!.next_run)}.`}
            <Show when={task()!.retry}>{(r) => ` Retry ${r().attempt} at ${when(r().at)}.`}</Show>
          </p>
        </Show>

        <fieldset class="field-card">
          <legend>About</legend>
          <div class="form-grid">
            <Show when={isNew()}>
              <label for="task-name">Name</label>
              <div>
                <input
                  id="task-name"
                  value={name()}
                  placeholder="morning_digest"
                  onInput={(e) => setName(e.currentTarget.value)}
                />
                <p class="muted hint">a-z, 0-9 and _. Saved as tasks/{name() || "<name>"}.wcl.</p>
              </div>
            </Show>
            <label for="task-label">Label</label>
            <Text id="task-label" item="label" placeholder="Morning digest" />
            <label for="task-description">Description</label>
            <Text id="task-description" item="description" />
          </div>
        </fieldset>

        <fieldset class="field-card">
          <legend>Schedule</legend>
          <div class="form-grid">
            <label for="task-preset">Repeat</label>
            <div class="scheduled-preset">
              <select
                id="task-preset"
                value={choice().preset}
                onChange={(e) => choose(e.currentTarget.value as Preset)}
              >
                <option value="hourly">Every hour</option>
                <option value="daily">Every day at…</option>
                <option value="weekdays">Weekdays at…</option>
                <option value="weekly">Every week on…</option>
                <option value="custom">Custom cron schedule</option>
              </select>
              <Show when={choice().preset === "weekly"}>
                <select
                  aria-label="Day of the week"
                  value={String(choice().day)}
                  onChange={(e) => choose("weekly", choice().time, Number(e.currentTarget.value))}
                >
                  <For each={DAYS}>{(d, i) => <option value={String(i())}>{d}</option>}</For>
                </select>
              </Show>
              <Show when={["daily", "weekdays", "weekly"].includes(choice().preset)}>
                <input
                  type="time"
                  aria-label="Time of day"
                  value={choice().time}
                  onChange={(e) => choose(choice().preset, e.currentTarget.value || "07:00")}
                />
              </Show>
            </div>
            <label for="task-schedule">Cron schedule</label>
            <div>
              <input
                id="task-schedule"
                class="mono"
                value={model.schedule}
                placeholder="0 7 * * 1-5"
                aria-describedby="schedule-issue"
                onInput={(e) => {
                  setModel("schedule", e.currentTarget.value);
                  setStatus(null);
                }}
              />
              <p class="muted hint">Minute, hour, day of month, month, day of week.</p>
              <FieldIssues issues={issues()} at="schedule" id="schedule-issue" />
            </div>
            <label for="task-timezone">Time zone</label>
            <div>
              <input
                id="task-timezone"
                list="task-timezones"
                value={model.timezone}
                placeholder="UTC"
                onInput={(e) => setModel("timezone", e.currentTarget.value)}
              />
              <datalist id="task-timezones">
                <For each={zoneList}>{(z) => <option value={z} />}</For>
              </datalist>
              <FieldIssues issues={issues()} at="timezone" />
            </div>
          </div>
          <div class="scheduled-preview" aria-live="polite">
            <Show
              when={preview()}
              fallback={
                <Show when={previewError()}>
                  <p class="error" role="alert" data-testid="schedule-error">
                    {previewError()}
                  </p>
                </Show>
              }
            >
              {(p) => (
                <>
                  <p class="scheduled-words" data-testid="schedule-text">
                    {p().text}
                  </p>
                  <p class="muted hint">Next runs, in {p().timezone}:</p>
                  <ol class="scheduled-next" aria-label="Next runs">
                    <For each={p().next}>{(ms) => <li>{inZone(ms, p().timezone)}</li>}</For>
                  </ol>
                </>
              )}
            </Show>
          </div>
        </fieldset>

        <fieldset class="field-card">
          <legend>Script</legend>
          <p class="muted hint">
            wscript, saved as scripts/{scriptName() || "<name>"}.ws. The <code>vault</code>, <code>http</code>,{" "}
            <code>fs</code> and <code>secrets</code> modules are available.
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
          <Show when={taskProblemsOfScript().length > 0}>
            <ul class="error" role="alert" aria-label="Script problems">
              <For each={taskProblemsOfScript()}>{(p) => <li>{p.message}</li>}</For>
            </ul>
          </Show>
          <div class="form-grid">
            <label for="task-function">Function</label>
            <div>
              <Text id="task-function" item="function" placeholder="run" />
              <FieldIssues issues={issues()} at="function" />
              <FieldIssues issues={issues()} at="script" />
            </div>
          </div>
        </fieldset>

        <fieldset class="field-card">
          <legend>Permissions</legend>
          <div class="form-grid">
            <span id="web-access-label">Web access</span>
            <div role="radiogroup" aria-labelledby="web-access-label" class="scheduled-radios">
              <For
                each={
                  [
                    ["none", "No web access"],
                    ["hosts", "Only these hosts"],
                    ["any", "Any web address"],
                  ] as [WebAccess, string][]
                }
              >
                {([value, label]) => (
                  <label>
                    <input
                      type="radio"
                      name="web-access"
                      value={value}
                      checked={web() === value}
                      onChange={() => setWeb(value)}
                    />{" "}
                    {label}
                  </label>
                )}
              </For>
              <Show when={web() === "hosts"}>
                <textarea
                  aria-label="Allowed hosts, one per line"
                  rows={2}
                  placeholder="api.example.com"
                  value={hosts()}
                  onInput={(e) => setHosts(e.currentTarget.value)}
                />
              </Show>
              <Show when={web() === "any"}>
                <p class="muted hint">The script may call any http or https address, including ones on your network.</p>
              </Show>
              <FieldIssues issues={issues()} at="allow_domains" />
            </div>
            <span id="files-label">Files</span>
            <div role="group" aria-labelledby="files-label">
              <Show when={dirs.length === 0}>
                <p class="muted hint">No file access. Add a directory to let the script read or write files in it.</p>
              </Show>
              <Index each={dirs}>
                {(d, i) => (
                  <div class="list-row" data-testid={`dir-${i}`}>
                    <input
                      aria-label={`Directory ${i + 1}`}
                      class="mono"
                      placeholder="/home/me/exports"
                      value={d().path}
                      onInput={(e) => setDirs(i, "path", e.currentTarget.value)}
                    />
                    <select
                      aria-label={`Access to directory ${i + 1}`}
                      value={d().write ? "write" : "read"}
                      onChange={(e) => setDirs(i, "write", e.currentTarget.value === "write")}
                    >
                      <option value="read">Read only</option>
                      <option value="write">Read and write</option>
                    </select>
                    <button
                      type="button"
                      aria-label={`Remove directory ${i + 1}`}
                      onClick={() => setDirs((all) => all.filter((_, j) => j !== i))}
                    >
                      Remove
                    </button>
                  </div>
                )}
              </Index>
              <button type="button" onClick={() => setDirs(dirs.length, { path: "", write: false })}>
                Add directory
              </button>
              <p class="muted hint">The vault's own folder is always refused: scripts change notes through vault.</p>
              <FieldIssues issues={issues()} at="read_paths" />
              <FieldIssues issues={issues()} at="write_paths" />
            </div>
            <span id="secrets-label">Secrets</span>
            <div role="group" aria-labelledby="secrets-label">
              <Show
                when={secretChoices().length > 0}
                fallback={<p class="muted hint">There are no secrets yet.</p>}
              >
                <div class="flags">
                  <For each={secretChoices()}>
                    {(s) => (
                      <label>
                        <input
                          type="checkbox"
                          checked={model.secrets.includes(s)}
                          onChange={(e) =>
                            setModel(
                              "secrets",
                              e.currentTarget.checked ? [...model.secrets, s] : model.secrets.filter((x) => x !== s),
                            )
                          }
                        />{" "}
                        {s}
                      </label>
                    )}
                  </For>
                </div>
              </Show>
              <p class="muted hint">
                The script reads them with secrets::get. <A href="/settings/secrets">Set secret values</A>
              </p>
              <FieldIssues issues={issues()} at="secrets" />
            </div>
          </div>
        </fieldset>

        <fieldset class="field-card">
          <legend>Running</legend>
          <div class="form-grid">
            <label for="task-retries">Retries</label>
            <div>
              <input
                id="task-retries"
                type="number"
                min="0"
                step="1"
                value={model.retries}
                onInput={(e) => setModel("retries", e.currentTarget.valueAsNumber)}
              />
              <FieldIssues issues={issues()} at="retries" />
            </div>
            <label for="task-retry-delay">First retry after</label>
            <div>
              <Text id="task-retry-delay" item="retry_delay" placeholder="PT1M" />
              <FieldIssues issues={issues()} at="retry_delay" />
            </div>
            <label for="task-timeout">Time limit</label>
            <div>
              <Text id="task-timeout" item="timeout" placeholder="PT30S" />
              <FieldIssues issues={issues()} at="timeout" />
            </div>
            <span />
            <label>
              <input
                type="checkbox"
                checked={model.paused}
                onChange={(e) => setModel("paused", e.currentTarget.checked)}
              />{" "}
              Paused: run only when run by hand
            </label>
          </div>
        </fieldset>

        <div class="save-bar">
          <Show when={localIssues().length > 0}>
            <ul class="error" role="alert" aria-label="Problems">
              <For each={localIssues()}>{(p) => <li>{p}</li>}</For>
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
          <button
            type="button"
            class="primary"
            disabled={busy() || localIssues().length > 0}
            onClick={() => void save()}
          >
            {isNew() ? "Create" : "Save"}
          </button>
        </div>
        <Show when={!isNew()}>
          <h2>Run history</h2>
          <p class="muted hint">Newest first, up to the latest 50 runs. Open a run to see its log.</p>
          <Show when={runsUnavailable()}>
            <p class="muted" role="note">
              The run history is your server's: {runsUnavailable()}.
            </p>
          </Show>
          <Show
            when={runs().length > 0}
            fallback={<Show when={!runsUnavailable()}><p class="muted">No runs yet.</p></Show>}
          >
            <ol class="run-list" aria-label="Runs">
              <For each={runs()}>{(r) => <RunItem run={r} open={r.id === justRan()} />}</For>
            </ol>
          </Show>
        </Show>
      </Show>
    </section>
  );
}
