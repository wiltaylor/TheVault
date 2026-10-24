// The agent screen's Triggers tab (agents G5, G9 to G12): the agent's triggers, each a schedule
// (with its next run times, as Settings > Scheduled shows them) or a note event (the changes,
// the types and a `where` condition), with the prompt each run starts from and its last runs,
// each linking to the thread it wrote. The triggers are part of the agent's draft, so the tab's
// Save writes the whole agent, as Settings and Permissions do. A trigger run has no one to ask,
// so the tab lists the tools such a run is offered. A mention trigger (agents G16) answers calls,
// `@<name>` written in a note of the types it names, and warns when the agent could not change
// the note it is called from.

import { A } from "@solidjs/router";
import { For, Index, Show, createEffect, createResource, createSignal, on, onCleanup } from "solid-js";
import { type AgentRun, type AgentTrigger, type CronPreview, api } from "../api";
import { toolGroups } from "../agents";
import { types } from "../session";
import { type AgentTabProps, SaveBar } from "./AgentScreen";
import { Outcome, inZone, zones } from "./Scheduled";
import { errorText } from "./TypeEditor";

/** The note changes an event trigger may name, with their labels. */
export const TRIGGER_OPS: [string, string][] = [
  ["create", "Created"],
  ["update", "Changed"],
  ["delete", "Moved to the trash"],
];

/** A trigger name not yet in `taken`: `base`, then `base_2`, `base_3` and on. */
export function freshTriggerName(base: string, taken: readonly string[]): string {
  if (!taken.includes(base)) return base;
  for (let i = 2; ; i++) {
    const name = `${base}_${i}`;
    if (!taken.includes(name)) return name;
  }
}

/** The legend of each kind of trigger. */
const KIND_LEGENDS: Record<AgentTrigger["kind"], string> = {
  schedule: "On a schedule",
  event: "When notes change",
  mention: "When @mentioned in a note",
};

/** The warning a mention trigger shows when the agent cannot change the note it is called from. */
export const MENTION_WARNING =
  "This agent cannot change the note it is called from: turn off Ask before creating and editing notes on its Permissions tab.";

/** Whether an agent with these permissions can change a note itself, with no one to confirm. */
export function changesNotes(p: AgentTabProps["draft"]["permissions"]): boolean {
  return !p.confirm.notes && (p.tools === null || p.tools.includes("propose_edit"));
}

/** The browser's IANA time zone, a new schedule's default. */
function browserZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** How long a run took, once it ended. */
function took(run: AgentRun): string {
  return run.ended === null ? "" : `${((run.ended - run.started) / 1000).toFixed(1)} s`;
}

/** When a run started, in the browser's time. */
function startedAt(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** A schedule in words and its next runs, from the server, a moment after typing stops. */
function SchedulePreview(props: { schedule: string; timezone: string }) {
  const [preview, setPreview] = createSignal<CronPreview | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  createEffect(
    on([() => props.schedule, () => props.timezone], ([schedule, zone]) => {
      clearTimeout(timer);
      const mine = ++seq;
      if (!schedule.trim()) {
        setPreview(null);
        setError("Enter a schedule.");
        return;
      }
      timer = setTimeout(async () => {
        try {
          const p = await api.cron(schedule.trim(), zone.trim() || "UTC");
          if (mine !== seq) return;
          setPreview(p);
          setError(null);
        } catch (e) {
          if (mine !== seq) return;
          setPreview(null);
          setError(errorText(e).replace(/^(schedule|timezone): /, ""));
        }
      }, 200);
    }),
  );
  onCleanup(() => clearTimeout(timer));
  return (
    <div class="scheduled-preview" aria-live="polite">
      <Show
        when={preview()}
        fallback={
          <Show when={error()}>
            <p class="error" role="alert">
              {error()}
            </p>
          </Show>
        }
      >
        {(p) => (
          <>
            <p class="scheduled-words">{p().text}</p>
            <p class="muted hint">Next runs, in {p().timezone}:</p>
            <ol class="scheduled-next" aria-label="Next runs">
              <For each={p().next}>{(ms) => <li>{inZone(ms, p().timezone)}</li>}</For>
            </ol>
          </>
        )}
      </Show>
    </div>
  );
}

/** One trigger's last runs, each linking to its thread. */
function RunList(props: { agent: string; runs: AgentRun[] | undefined; loading: boolean }) {
  return (
    <div class="agent-trigger-runs">
      <h4>Last runs</h4>
      <Show
        when={(props.runs ?? []).length > 0}
        fallback={<p class="muted">{props.loading ? "Reading the runs…" : "No runs yet."}</p>}
      >
        <ol class="agent-run-list" aria-label="Last runs">
          <For each={props.runs}>
            {(r) => (
              <li class="agent-run">
                <Outcome outcome={r.outcome} /> <span>{startedAt(r.started)}</span>{" "}
                <Show when={took(r)}>
                  <span class="muted">{took(r)}</span>{" "}
                </Show>
                <Show when={r.thread}>
                  <A href={`/agents/${encodeURIComponent(props.agent)}?thread=${encodeURIComponent(r.thread!)}`}>
                    Open thread
                  </A>
                </Show>
                <Show when={r.message}>
                  <div class="agent-run-message">{r.message}</div>
                </Show>
              </li>
            )}
          </For>
        </ol>
      </Show>
    </div>
  );
}

/** The Triggers tab. */
export function TriggersTab(p: AgentTabProps) {
  const [runs, { refetch }] = createResource(
    () => ({ name: p.agent.name, wcl: p.agent.wcl }),
    async ({ name }) => {
      try {
        return { ok: await api.agentRuns(name), error: null as string | null };
      } catch (e) {
        return { ok: null, error: errorText(e) };
      }
    },
  );
  const zoneList = zones();
  const triggers = () => p.draft.triggers;
  const setTriggers = (f: (t: AgentTrigger[]) => AgentTrigger[]) => p.setDraft((d) => ({ ...d, triggers: f(d.triggers) }));
  const setAt = (i: number, patch: Partial<AgentTrigger>) =>
    setTriggers((ts) => ts.map((t, j) => (j === i ? ({ ...t, ...patch } as AgentTrigger) : t)));
  const names = () => triggers().map((t) => t.name);

  function add(kind: AgentTrigger["kind"]) {
    const t: AgentTrigger =
      kind === "schedule"
        ? { name: freshTriggerName("scheduled", names()), kind, schedule: "0 7 * * *", timezone: browserZone(), prompt: "" }
        : kind === "event"
          ? { name: freshTriggerName("on_change", names()), kind, on: ["create"], type: [], where: null, prompt: "" }
          : { name: freshTriggerName("asked", names()), kind, type: [], prompt: "" };
    setTriggers((ts) => [...ts, t]);
  }

  function toggleOp(i: number, op: string, on: boolean) {
    const t = triggers()[i];
    if (t?.kind !== "event") return;
    const next = on ? [...t.on.filter((o) => o !== op), op] : t.on.filter((o) => o !== op);
    setAt(i, { on: TRIGGER_OPS.map(([o]) => o).filter((o) => next.includes(o)) });
  }

  function toggleType(i: number, name: string, on: boolean) {
    const t = triggers()[i];
    if (t?.kind !== "event" && t?.kind !== "mention") return;
    setAt(i, { type: on ? [...t.type.filter((n) => n !== name), name] : t.type.filter((n) => n !== name) });
  }

  const runsOf = (name: string) => runs()?.ok?.triggers.find((t) => t.name === name)?.runs;
  const typeNames = () => [...types()].sort((a, b) => (a.label || a.name).localeCompare(b.label || b.name));
  const offered = () => {
    const tools = runs()?.ok?.tools;
    if (!tools) return null;
    return toolGroups(tools).flatMap((g) => g.tools.map((t) => t.label));
  };
  const id = (i: number, what: string) => `${p.id}-trigger-${i}-${what}`;

  async function save() {
    await p.save();
    void refetch();
  }

  return (
    <section class="agent-tab" aria-label="Triggers">
      <p class="muted">
        A trigger runs {p.agent.label || p.agent.name} on its own: on a schedule, when notes change, or when a note calls
        it. Each run starts a new thread whose first message is the trigger's prompt. A note change made by an agent
        never starts a run.
      </p>
      <Show when={!p.agent.enabled}>
        <p class="agent-perm-note">This agent is disabled, so none of its triggers run.</p>
      </Show>
      <Show when={runs()?.error}>
        <p class="error" role="alert">
          The runs could not be read: {runs()!.error}
        </p>
      </Show>
      <Show when={offered()}>
        {(tools) => (
          <p class="agent-trigger-tools">
            <span class="muted">With no one to ask, a run is offered only the tools that need no confirmation: </span>
            {tools().length > 0 ? tools().join(", ") : "none"}.
          </p>
        )}
      </Show>
      <fieldset class="agent-fieldset" disabled={p.readOnly}>
        <Show when={triggers().length === 0}>
          <p class="muted">No triggers yet.</p>
        </Show>
        <Index each={triggers()}>
          {(t, i) => (
            <fieldset class="field-card agent-trigger" aria-label={`Trigger ${t().name}`}>
              <legend>{KIND_LEGENDS[t().kind]}</legend>
              <div class="form-grid">
                <label for={id(i, "name")}>Name</label>
                <input
                  id={id(i, "name")}
                  spellcheck={false}
                  value={t().name}
                  onChange={(e) => setAt(i, { name: e.currentTarget.value.trim() })}
                />
                <Show when={t().kind === "schedule" ? (t() as Extract<AgentTrigger, { kind: "schedule" }>) : null}>
                  {(s) => (
                    <>
                      <label for={id(i, "schedule")}>Schedule</label>
                      <input
                        id={id(i, "schedule")}
                        spellcheck={false}
                        placeholder="0 7 * * *"
                        value={s().schedule}
                        onInput={(e) => setAt(i, { schedule: e.currentTarget.value })}
                      />
                      <label for={id(i, "timezone")}>Time zone</label>
                      <input
                        id={id(i, "timezone")}
                        list={id(i, "zones")}
                        placeholder="UTC"
                        value={s().timezone}
                        onInput={(e) => setAt(i, { timezone: e.currentTarget.value })}
                      />
                      <datalist id={id(i, "zones")}>
                        <For each={zoneList}>{(z) => <option value={z} />}</For>
                      </datalist>
                    </>
                  )}
                </Show>
              </div>
              <Show when={t().kind === "schedule" ? (t() as Extract<AgentTrigger, { kind: "schedule" }>) : null}>
                {(s) => <SchedulePreview schedule={s().schedule} timezone={s().timezone} />}
              </Show>
              <Show when={t().kind === "event" ? (t() as Extract<AgentTrigger, { kind: "event" }>) : null}>
                {(e) => (
                  <>
                    <div class="agent-check-row" role="group" aria-label="Changes">
                      <span>Runs when a note is</span>
                      <For each={TRIGGER_OPS}>
                        {([op, label]) => (
                          <label class="agent-check">
                            <input
                              type="checkbox"
                              checked={e().on.includes(op)}
                              onChange={(ev) => toggleOp(i, op, ev.currentTarget.checked)}
                            />{" "}
                            {label}
                          </label>
                        )}
                      </For>
                    </div>
                    <Show when={e().on.length === 0}>
                      <p class="error">Tick at least one change.</p>
                    </Show>
                    <div class="agent-type-list" role="group" aria-label="Note types">
                      <span>
                        Of type <span class="muted">{e().type.length === 0 ? "(none ticked: every type)" : ""}</span>
                      </span>
                      <For each={typeNames()}>
                        {(ty) => (
                          <label class="agent-check">
                            <input
                              type="checkbox"
                              checked={e().type.includes(ty.name)}
                              onChange={(ev) => toggleType(i, ty.name, ev.currentTarget.checked)}
                            />{" "}
                            {ty.label || ty.name}
                          </label>
                        )}
                      </For>
                    </div>
                    <label for={id(i, "where")}>Where</label>
                    <input
                      id={id(i, "where")}
                      class="agent-where"
                      spellcheck={false}
                      placeholder={'status == "open"'}
                      value={e().where ?? ""}
                      onInput={(ev) => setAt(i, { where: ev.currentTarget.value || null })}
                    />
                    <p class="muted">
                      A condition in the query language, checked against the note after the change, or before it for a
                      move to the trash. Empty runs for every note. Changes to one note within 30 seconds start one run.
                    </p>
                  </>
                )}
              </Show>
              <Show when={t().kind === "mention" ? (t() as Extract<AgentTrigger, { kind: "mention" }>) : null}>
                {(m) => (
                  <>
                    <p class="muted">
                      Write <code>@{p.agent.name}</code> in a note to call this agent, or choose “Ask{" "}
                      {p.agent.label || p.agent.name}” after typing <kbd>@</kbd>. Once the note has been idle for 5 seconds,
                      the call is replaced with a link to a new thread, and the run starts with the paragraph that held
                      the call.
                    </p>
                    <Show when={!changesNotes(p.draft.permissions)}>
                      <p class="agent-perm-note" role="note">
                        {MENTION_WARNING}
                      </p>
                    </Show>
                    <div class="agent-type-list" role="group" aria-label="Note types">
                      <span>
                        Called from notes of type{" "}
                        <span class="muted">{m().type.length === 0 ? "(none ticked: every type)" : ""}</span>
                      </span>
                      <For each={typeNames()}>
                        {(ty) => (
                          <label class="agent-check">
                            <input
                              type="checkbox"
                              checked={m().type.includes(ty.name)}
                              onChange={(ev) => toggleType(i, ty.name, ev.currentTarget.checked)}
                            />{" "}
                            {ty.label || ty.name}
                          </label>
                        )}
                      </For>
                    </div>
                  </>
                )}
              </Show>
              <label for={id(i, "prompt")}>Prompt</label>
              <textarea
                id={id(i, "prompt")}
                class="ai-system-prompt agent-trigger-prompt"
                rows={4}
                value={t().prompt}
                placeholder={
                  t().kind === "mention"
                    ? "Optional: what the agent is told before the text from the note."
                    : "What the agent is asked to do on each run."
                }
                onInput={(e) => setAt(i, { prompt: e.currentTarget.value })}
              />
              <Show when={!p.readOnly}>
                <div class="actions">
                  <button type="button" onClick={() => setTriggers((ts) => ts.filter((_, j) => j !== i))}>
                    Remove trigger
                  </button>
                </div>
              </Show>
              <RunList agent={p.agent.name} runs={runsOf(t().name)} loading={runs.loading} />
            </fieldset>
          )}
        </Index>
        <Show when={!p.readOnly}>
          <div class="actions">
            <button type="button" onClick={() => add("schedule")}>
              Add a schedule
            </button>
            <button type="button" onClick={() => add("event")}>
              Add a note event
            </button>
            <button type="button" onClick={() => add("mention")}>
              Add a mention
            </button>
          </div>
        </Show>
      </fieldset>
      <SaveBar dirty={p.dirty} readOnly={p.readOnly} save={save} />
    </section>
  );
}
