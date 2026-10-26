// The agent screen, `/agents/<name>` (agents G5). Its main view is the agent's chat (agents G7,
// `ai/AgentChat.tsx`): a bar with the session picker, Rename, Archive and Delete for the chosen
// session, and the settings cog at its end; then the chosen session, filling the pane.
// `?thread=<id>` opens one, and the address follows the session shown. The cog opens the agent's
// settings at `/agents/<name>/settings`, as tabs at `/agents/<name>/settings/<tab>`: General holds
// its label, icon, model and system prompt; Permissions its own tools, confirmation and internet
// switches and note types (agents G4); Memory what it remembers (agents G21, `AgentMemory.tsx`),
// which the apps can change too; Triggers its schedules and note events with their last runs
// (agents G9 to G12, `AgentTriggers.tsx`); File its WCL. General, Permissions and Triggers edit
// one draft, so moving between them, or to the chat and back, keeps unsaved changes, and each
// tab's Save writes the whole agent. Show in log opens the agent's AI activity in the Log
// (activity Y5). The addresses of the tabs the screen had before, `/agents/<name>/chat`,
// `/memory`, `/permissions`, `/triggers` and `/file`, lead to their new places. In the apps the
// settings are read only: agent files need `admin`, which a device token never has (G2).

import { A, useLocation, useNavigate, useParams, useSearchParams } from "@solidjs/router";
import {
  For,
  type JSX,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  createUniqueId,
  on,
  onCleanup,
  onMount,
} from "solid-js";
import {
  AGENT_ICON,
  AGENT_PROMPT_MAX,
  CONFIRM_SWITCHES,
  WEB_SWITCHES,
  agentForm,
  agentLabel,
  isDefaultSubagents,
  agentTools,
  agents,
  agentsVersion,
  refreshAgents,
  toolGroups,
} from "../agents";
import {
  type Agent,
  type AgentEffort,
  type AgentForm,
  type AgentSubagents,
  type AiAccounts,
  type AiModel,
  type AiStatus,
  ApiError,
  api,
} from "../api";
import { AgentChat } from "../ai/AgentChat";
import { accountFor, accountNamed } from "../ai/accounts";
import { logHref } from "../activity/model";
import { IconPicker } from "../components/IconPicker";
import { TypeIcon } from "../components/Icon";
import { Spinner } from "../components/SyncStatus";
import { appInfo } from "../inApp";
import { types } from "../session";
import { refreshSkills, skills } from "../skills";
import { usePane } from "../workspace/pane";
import { MemoryTab } from "./AgentMemory";
import { TriggersTab } from "./AgentTriggers";
import { modelLabel } from "./Ai";
import { errorText } from "./TypeEditor";

export const EFFORTS: [AgentEffort, string][] = [
  ["low", "Low"],
  ["medium", "Medium"],
  ["high", "High"],
  ["xhigh", "Extra high"],
  ["max", "Max"],
];

/** What every tab is given: the saved agent, the draft the form edits, and the ways to save. */
export interface AgentTabProps {
  agent: Agent;
  draft: AgentForm;
  setDraft: (f: (d: AgentForm) => AgentForm) => void;
  /** Whether the draft differs from the saved agent. */
  dirty: boolean;
  /** Write the draft as the agent. */
  save: () => Promise<void>;
  /** Write `wcl` as the agent's file. */
  saveWcl: (wcl: string) => Promise<void>;
  /** Settings > AI as it stands, in the web app; null in the apps or before it loads. */
  ai: AiStatus | null;
  /** The AI accounts, from `GET /api/ai/accounts` in every client (accounts A7); null before they load or when they cannot be read. */
  accounts: AiAccounts | null;
  /** A prefix for element ids, unique to this screen. */
  id: string;
}

/**
 * The settings screen's tabs, in order; the first is the default. Tickets that add a tab add it
 * here.
 */
export const AGENT_TABS: { id: string; label: string; render: (p: AgentTabProps) => JSX.Element }[] = [
  { id: "general", label: "General", render: (p) => <SettingsTab {...p} /> },
  { id: "permissions", label: "Permissions", render: (p) => <PermissionsTab {...p} /> },
  {
    id: "memory",
    label: "Memory",
    render: (p) => <MemoryTab agent={p.agent.name} label={agentLabel(p.agent)} id={p.id} />,
  },
  { id: "triggers", label: "Triggers", render: (p) => <TriggersTab {...p} /> },
  { id: "file", label: "File", render: (p) => <FileTab {...p} /> },
];

/** Where the chat of agent `name` is, open on thread `thread` when one is given. */
export function agentHref(name: string, thread?: string | null): string {
  return `/agents/${encodeURIComponent(name)}${thread ? `?thread=${encodeURIComponent(thread)}` : ""}`;
}

/** Where tab `tab` of the settings of agent `name` is; the first tab when none is given. */
export function agentSettingsHref(name: string, tab?: string): string {
  const base = `/agents/${encodeURIComponent(name)}/settings`;
  return tab && tab !== AGENT_TABS[0]!.id ? `${base}/${tab}` : base;
}

/**
 * Where an address of the agent screen leads, as `/agents/<name>/<section>/<tab>` gives it: the
 * chat, a settings tab, or another address to go to in its place, for the screen's old tabs.
 */
export function agentView(
  name: string,
  section: string | undefined,
  tab: string | undefined,
  search: string,
): { view: "chat" } | { view: "settings"; tab: string } | { redirect: string } {
  if (!section) return tab ? { redirect: `${agentHref(name)}${search}` } : { view: "chat" };
  if (section === "chat") return { redirect: `${agentHref(name)}${search}` };
  if (section === "settings") {
    if (!tab) return { view: "settings", tab: AGENT_TABS[0]!.id };
    if (AGENT_TABS.some((t) => t.id === tab)) return { view: "settings", tab };
    return { redirect: agentSettingsHref(name) };
  }
  // The screen's old tabs: Memory, Permissions, Triggers and File are settings now.
  if (!tab && AGENT_TABS.some((t) => t.id === section)) return { redirect: agentSettingsHref(name, section) };
  return { redirect: agentHref(name) };
}

/** A Save button with its spinner, and the result of the last save below it. */
export function SaveBar(props: { dirty: boolean; readOnly?: boolean; save: () => Promise<void>; label?: string }) {
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  async function run() {
    setError(null);
    setNotice(null);
    setPending(true);
    try {
      await props.save();
      setNotice("Saved.");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <Show when={!props.readOnly}>
      <div class="actions">
        <button
          type="button"
          class="primary"
          disabled={pending() || !props.dirty}
          aria-busy={pending() ? "true" : undefined}
          onClick={() => void run()}
        >
          <Show when={pending()}>
            <Spinner />{" "}
          </Show>
          {props.label ?? "Save"}
        </button>
        <Show when={props.dirty}>
          <span class="muted">Unsaved changes</span>
        </Show>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice() && !props.dirty}>
        <p class="muted" role="status">
          {notice()}
        </p>
      </Show>
    </Show>
  );
}

/**
 * The model: chosen from the account's live list, with the account's default first, or typed
 * when the list cannot be read (ai D6, D24). Empty means the account's default model.
 */
export function ModelPicker(props: {
  id: string;
  /** The account's id. */
  account: string | null;
  value: string | null;
  onChange: (model: string | null) => void;
}) {
  const [list] = createResource(
    () => props.account,
    async (id) => {
      try {
        const l = await api.aiModels(id);
        return { models: l.models, model: l.model, error: null as string | null };
      } catch (e) {
        return { models: [] as AiModel[], model: null, error: errorText(e) };
      }
    },
  );
  const typed = () => !props.account || !!list()?.error;
  const defaultLabel = () => {
    const m = list()?.model;
    return m ? `Account default (${m})` : "Account default";
  };
  return (
    <>
      <Show
        when={!typed()}
        fallback={
          <>
            <input
              id={props.id}
              list={`${props.id}-ids`}
              autocomplete="off"
              spellcheck={false}
              placeholder="Account default"
              value={props.value ?? ""}
              onChange={(e) => props.onChange(e.currentTarget.value.trim() || null)}
            />
            <datalist id={`${props.id}-ids`}>
              <For each={list()?.models ?? []}>{(m) => <option value={m.id} />}</For>
            </datalist>
          </>
        }
      >
        <Show when={list()} fallback={<span class="muted">Listing the models…</span>}>
          {(l) => (
            <select
              id={props.id}
              value={props.value ?? ""}
              onChange={(e) => props.onChange(e.currentTarget.value || null)}
            >
              <option value="">{defaultLabel()}</option>
              <Show when={props.value && !l().models.some((m) => m.id === props.value)}>
                <option value={props.value!}>{props.value}</option>
              </Show>
              <For each={l().models}>{(m) => <option value={m.id}>{modelLabel(m)}</option>}</For>
            </select>
          )}
        </Show>
      </Show>
      <Show when={typed()}>
        <p class="muted agent-model-note">
          {props.account
            ? `The model list could not be read${list()?.error ? ` (${list()!.error})` : ""}: type a model id, or leave it empty for the account's default.`
            : "No AI account is set up in Settings > AI: add one, or type a model id."}
        </p>
      </Show>
    </>
  );
}

function SettingsTab(p: AgentTabProps) {
  const id = (what: string) => `${p.id}-${what}`;
  const set = (patch: Partial<AgentForm>) => p.setDraft((d) => ({ ...d, ...patch }));
  const accounts = () => p.accounts?.accounts ?? [];
  const defaultAccount = () => accountFor(p.accounts, null);
  /** The account the agent runs on (accounts A5): the one it names, else the default. */
  const account = () => accountFor(p.accounts, p.draft.account);
  /** The account the agent names when no account has that name (accounts A6). */
  const missing = () => (p.accounts && p.draft.account && !accountNamed(accounts(), p.draft.account) ? p.draft.account : null);
  const prompt = () => p.draft.system_prompt ?? "";
  const length = () => [...prompt()].length;
  return (
    <section class="agent-tab" aria-label="General">
      <fieldset class="agent-fieldset">
        <div class="form-grid">
          <label for={id("label")}>Label</label>
          <input id={id("label")} value={p.draft.label} onInput={(e) => set({ label: e.currentTarget.value })} />
          <label for={id("description")}>Description</label>
          <input
            id={id("description")}
            value={p.draft.description ?? ""}
            onInput={(e) => set({ description: e.currentTarget.value || null })}
          />
          <label for={id("icon")}>Icon</label>
          <div>
            <IconPicker id={id("icon")} value={p.draft.icon ?? ""} onChange={(icon) => set({ icon: icon || null })} />
          </div>
          <span>Enabled</span>
          <label class="ai-confirm-switch">
            <input
              type="checkbox"
              role="switch"
              checked={p.draft.enabled}
              disabled={p.agent.built_in}
              onChange={(e) => set({ enabled: e.currentTarget.checked })}
            />{" "}
            {p.agent.built_in
              ? `${agentLabel(p.agent)} is built in and always enabled.`
              : "Enabled: off keeps the file but runs no trigger and starts no chat."}
          </label>
          <label for={id("account")}>Account</label>
          <div>
            <select id={id("account")} onChange={(e) => set({ account: e.currentTarget.value || null, model: null })}>
              <option value="" selected={!p.draft.account}>
                {defaultAccount() ? `Default account (${defaultAccount()!.name})` : "Default account"}
              </option>
              <For each={accounts()}>
                {(a) => (
                  <option value={a.name} selected={accountNamed([a], p.draft.account) !== undefined}>
                    {a.name}
                  </option>
                )}
              </For>
              <Show when={missing()}>
                {(name) => (
                  <option value={name()} selected>
                    {name()} (missing)
                  </option>
                )}
              </Show>
            </select>
            <Show when={missing()}>
              {(name) => (
                <p class="error agent-account-missing" role="alert">
                  Account {name()} does not exist; this agent uses the default account.
                </p>
              )}
            </Show>
          </div>
          <label for={id("model")}>Model</label>
          <div>
            <ModelPicker
              id={id("model")}
              account={account()?.id ?? null}
              value={p.draft.model}
              onChange={(model) => set({ model })}
            />
          </div>
          <label for={id("effort")}>Effort</label>
          <select
            id={id("effort")}
            value={p.draft.effort ?? ""}
            onChange={(e) => set({ effort: (e.currentTarget.value || null) as AgentEffort | null })}
          >
            <option value="">Model default</option>
            <For each={EFFORTS}>{([v, l]) => <option value={v}>{l}</option>}</For>
          </select>
        </div>
        <label for={id("prompt")}>System prompt</label>
        <p class="muted">
          What the agent tells the model before every conversation. Empty uses the shipped prompt, shown greyed out. The
          open note and the tool list are added for you.
        </p>
        <textarea
          id={id("prompt")}
          class="ai-system-prompt agent-prompt"
          rows={16}
          spellcheck={false}
          placeholder={p.ai?.default_system_prompt ?? "The shipped chat prompt"}
          value={prompt()}
          onInput={(e) => set({ system_prompt: e.currentTarget.value || null })}
        />
        <div class="agent-prompt-foot">
          <span class={length() > AGENT_PROMPT_MAX ? "error" : "muted"}>
            {p.draft.system_prompt === null ? "Shipped prompt" : `${length()} of ${AGENT_PROMPT_MAX} characters`}
          </span>
          <button type="button" disabled={p.draft.system_prompt === null} onClick={() => set({ system_prompt: null })}>
            Use the shipped prompt
          </button>
        </div>
      </fieldset>
      <SaveBar dirty={p.dirty} save={p.save} />
    </section>
  );
}

/** What the Permissions tab warns of with Send from the Outbox without review on (outbox X9). */
export const OUTBOX_WARNING =
  "Emails, SMS messages and calendar invites this agent drafts are sent at once, in your name, without you seeing them first.";

/** What the Permissions tab warns of with Run wscript code on (script-tools T7). */
export const RUN_CODE_WARNING =
  "It can read and change anything in the vault, call any web host and read script secrets.";

/** What the Permissions tab says of the Browser automation switch (browser B6). */
export const BROWSER_NOTE =
  "Lets this agent run commands in the browser box and drive a browser as you watch. Web pages can try to instruct it.";

/** What the Permissions tab says of the Sub agents switch (subagents U2). */
export const SUBAGENTS_NOTE =
  "Lets this agent hand parts of a task to sub agents with start_subagent: throwaway copies of itself, or the agents ticked below. A sub agent never has more than this agent, and cannot start sub agents of its own. Only an admin turns this on.";

/**
 * The Permissions tab's sub agent settings (subagents U2): the switch, how many run at once, and
 * the agents it may delegate to by name, every agent but itself.
 */
function SubagentsSection(props: {
  agent: string;
  subagents: AgentSubagents | undefined;
  set: (s: AgentSubagents) => void;
}) {
  const now = (): AgentSubagents => props.subagents ?? { allow: false, max: 4, agents: [] };
  const others = createMemo(() =>
    agents()
      .filter((a) => a.name !== props.agent && !a.error)
      .sort((a, b) => agentLabel(a).localeCompare(agentLabel(b))),
  );
  // An agent the list names that the vault no longer has stays listed, to be unticked.
  const gone = () => now().agents.filter((n) => !others().some((a) => a.name === n));
  const toggle = (name: string, on: boolean) => {
    const rest = now().agents.filter((n) => n !== name);
    props.set({ ...now(), agents: on ? [...rest, name] : rest });
  };
  return (
    <div class="ai-confirm agent-subagents">
      <label class="ai-confirm-switch">
        <input
          type="checkbox"
          role="switch"
          checked={now().allow}
          onChange={(e) => props.set({ ...now(), allow: e.currentTarget.checked })}
        />{" "}
        Start sub agents
      </label>
      <p class="muted">{SUBAGENTS_NOTE}</p>
      <Show when={now().allow}>
        <label class="agent-check">
          <span>At most this many at once</span>
          <input
            type="number"
            min={1}
            max={8}
            value={now().max}
            aria-label="Sub agents at once"
            onChange={(e) => {
              const n = Math.round(Number(e.currentTarget.value));
              props.set({ ...now(), max: Number.isFinite(n) ? Math.min(8, Math.max(1, n)) : 4 });
            }}
          />
        </label>
        <fieldset class="agent-tool-group" aria-label="Agents it may delegate to">
          <legend>Agents it may delegate to</legend>
          <Show when={others().length > 0 || gone().length > 0} fallback={<p class="muted">There are no other agents.</p>}>
            <For each={others()}>
              {(a) => (
                <label class="agent-check">
                  <input type="checkbox" checked={now().agents.includes(a.name)} onChange={(e) => toggle(a.name, e.currentTarget.checked)} />{" "}
                  {agentLabel(a)} <code class="muted">{a.name}</code>
                </label>
              )}
            </For>
            <For each={gone()}>
              {(n) => (
                <label class="agent-check">
                  <input type="checkbox" checked onChange={(e) => toggle(n, e.currentTarget.checked)} /> <code>{n}</code>{" "}
                  <span class="muted">(no such agent)</span>
                </label>
              )}
            </For>
          </Show>
        </fieldset>
      </Show>
    </div>
  );
}

function PermissionsTab(p: AgentTabProps) {
  const perms = () => p.draft.permissions;
  const setPerms = (patch: Partial<AgentForm["permissions"]>) =>
    p.setDraft((d) => ({ ...d, permissions: { ...d.permissions, ...patch } }));
  // The default is no subagents block at all, as the agent file holds it only otherwise.
  const setSubagents = (sub: AgentSubagents) =>
    p.setDraft((d) => {
      const { subagents: _, ...rest } = d.permissions;
      return { ...d, permissions: isDefaultSubagents(sub) ? rest : { ...rest, subagents: sub } };
    });
  // The server's tools, and any the file names that this server no longer has.
  const all = createMemo(() => {
    const known = agentTools();
    return [...known, ...(perms().tools ?? []).filter((t) => !known.includes(t))];
  });
  const allowed = (tool: string) => perms().tools === null || perms().tools!.includes(tool);
  function toggleTool(tool: string, on: boolean) {
    const next = all().filter((t) => (t === tool ? on : allowed(t)));
    // Every tool ticked is no list at all, so a tool a later server offers is allowed too.
    const every = agentTools().length > 0 && agentTools().every((t) => next.includes(t));
    setPerms({ tools: every ? null : next });
  }
  const typeNames = createMemo(() => [...types()].sort((a, b) => (a.label || a.name).localeCompare(b.label || b.name)));
  function toggleType(name: string, on: boolean) {
    const now = perms().types ?? [];
    setPerms({ types: on ? [...now.filter((t) => t !== name), name] : now.filter((t) => t !== name) });
  }
  // The vault's skills (skills S6), and any the agent names that the vault no longer has.
  onMount(() => void refreshSkills());
  const skillRows = createMemo(() => {
    const listed = skills().map((s) => ({ name: s.name, valid: s.valid, missing: false }));
    const gone = (perms().skills ?? [])
      .filter((n) => !listed.some((s) => s.name === n))
      .map((name) => ({ name, valid: false, missing: true }));
    return [...listed, ...gone].sort((a, b) => a.name.localeCompare(b.name));
  });
  // Every skill is no list at all, which the form leaves out, as agentForm does.
  const setSkills = (list: string[] | null) =>
    p.setDraft((d) => {
      const { skills: _, ...rest } = d.permissions;
      return { ...d, permissions: list === null ? rest : { ...rest, skills: list } };
    });
  function toggleSkill(name: string, on: boolean) {
    const now = perms().skills ?? [];
    setSkills(on ? [...now.filter((s) => s !== name), name] : now.filter((s) => s !== name));
  }
  return (
    <section class="agent-tab" aria-label="Permissions">
      <p class="agent-perm-note">
        These permissions are {agentLabel(p.agent)}'s own: they replace Settings {">"} AI for this agent, and nothing is
        inherited. The person or token running a chat still limits what it can change.
      </p>
      <fieldset class="agent-fieldset">
        <h2>Tools</h2>
        <div class="agent-tool-groups">
          <For each={toolGroups(all())}>
            {(g) => (
              <fieldset class="agent-tool-group">
                <legend>{g.group}</legend>
                <For each={g.tools}>
                  {(t) => (
                    <label class="agent-check">
                      <input type="checkbox" checked={allowed(t.name)} onChange={(e) => toggleTool(t.name, e.currentTarget.checked)} />{" "}
                      {t.label} <code class="muted">{t.name}</code>
                    </label>
                  )}
                </For>
              </fieldset>
            )}
          </For>
        </div>
        <h2>Changes the agent makes</h2>
        <p class="muted">
          With an Ask switch on, the agent shows each change of that kind for you to apply or discard. With it off, it
          makes the change itself. A scheduled or event run has no one to ask, so it is not given a tool whose switch is on.
        </p>
        <div class="ai-confirm">
          <For each={CONFIRM_SWITCHES}>
            {(s) => (
              <label class="ai-confirm-switch">
                <input
                  type="checkbox"
                  role="switch"
                  checked={perms().confirm[s.key] ?? true}
                  onChange={(e) => setPerms({ confirm: { ...perms().confirm, [s.key]: e.currentTarget.checked } })}
                />{" "}
                {s.label}
              </label>
            )}
          </For>
        </div>
        <h3>Internet</h3>
        <div class="ai-confirm">
          <For each={WEB_SWITCHES}>
            {(s) => (
              <label class="ai-confirm-switch">
                <input
                  type="checkbox"
                  role="switch"
                  checked={perms().web[s.key]}
                  onChange={(e) => setPerms({ web: { ...perms().web, [s.key]: e.currentTarget.checked } })}
                />{" "}
                {s.label}
                <Show when={s.note}>
                  {" "}
                  <span class="muted">{s.note}</span>
                </Show>
              </label>
            )}
          </For>
        </div>
        <h3>Outbox</h3>
        <p class="muted">
          What the agent drafts with the Outbox tools waits in the Outbox's Draft tab until you mark it Ready.
        </p>
        <div class="ai-confirm">
          <label class="ai-confirm-switch">
            <input
              type="checkbox"
              role="switch"
              checked={perms().outbox?.send_without_review ?? false}
              onChange={(e) => setPerms({ outbox: { send_without_review: e.currentTarget.checked } })}
            />{" "}
            Send from the Outbox without review
          </label>
          <Show when={perms().outbox?.send_without_review}>
            <p class="agent-perm-note" role="note">
              {OUTBOX_WARNING}
            </p>
          </Show>
        </div>
        <h3>Scripts</h3>
        <div class="ai-confirm">
          <label class="ai-confirm-switch">
            <input
              type="checkbox"
              role="switch"
              checked={perms().scripts?.run_code ?? false}
              onChange={(e) => {
                const on = e.currentTarget.checked;
                // Off is no scripts block at all, as the agent file holds it only when on.
                p.setDraft((d) => {
                  const { scripts: _, ...rest } = d.permissions;
                  return { ...d, permissions: on ? { ...rest, scripts: { run_code: true } } : rest };
                });
              }}
            />{" "}
            Run wscript code
          </label>
          <p class="muted">Gives the agent the run_script tool, which runs wscript code it writes, with no limits on what it reaches.</p>
          <Show when={perms().scripts?.run_code}>
            <p class="agent-perm-note" role="note">
              {RUN_CODE_WARNING}
            </p>
          </Show>
        </div>
        <h3>Sub agents</h3>
        <SubagentsSection agent={p.agent.name} subagents={perms().subagents} set={setSubagents} />
        <h3>Browser</h3>
        <div class="ai-confirm">
          <label class="ai-confirm-switch">
            <input
              type="checkbox"
              role="switch"
              checked={perms().browser ?? false}
              onChange={(e) => setPerms({ browser: e.currentTarget.checked })}
            />{" "}
            Browser automation
          </label>
          <p class="muted">{BROWSER_NOTE}</p>
          <p class="muted">
            Whatever you are signed in to in the box's browser, this agent can reach. It needs the browser box (see
            Settings {">"} Integrations).
          </p>
        </div>
        <h2>Skills</h2>
        <p class="muted">
          The skills listed at the end of the agent's instructions, which it reads with use_skill. A skill's allowed-tools
          grants nothing: the tools above decide. A skill with errors is offered to no agent.
        </p>
        <div class="agent-types" role="radiogroup" aria-label="Skills">
          <label class="agent-check">
            <input
              type="radio"
              name={`${p.id}-skills`}
              checked={(perms().skills ?? null) === null}
              onChange={() => setSkills(null)}
            />{" "}
            All skills
          </label>
          <label class="agent-check">
            <input
              type="radio"
              name={`${p.id}-skills`}
              checked={(perms().skills ?? null) !== null}
              onChange={() => setSkills([])}
            />{" "}
            Only these skills
          </label>
        </div>
        <Show when={(perms().skills ?? null) !== null}>
          <div class="agent-type-list" role="group" aria-label="Allowed skills">
            <For each={skillRows()}>
              {(s) => (
                <label class="agent-check">
                  <input
                    type="checkbox"
                    checked={perms().skills!.includes(s.name)}
                    onChange={(e) => toggleSkill(s.name, e.currentTarget.checked)}
                  />{" "}
                  {s.name}
                  <Show when={s.missing}>
                    {" "}
                    <span class="muted">(not in the vault)</span>
                  </Show>
                  <Show when={!s.missing && !s.valid}>
                    {" "}
                    <span class="muted">(has errors)</span>
                  </Show>
                </label>
              )}
            </For>
          </div>
          <Show when={skillRows().length === 0}>
            <p class="muted">The vault has no skills yet: add them under Agents {">"} Skills in the sidebar.</p>
          </Show>
          <Show when={skillRows().length > 0 && perms().skills!.length === 0}>
            <p class="muted">With no skill ticked, the agent is offered none.</p>
          </Show>
        </Show>
        <h2>Note types</h2>
        <div class="agent-types" role="radiogroup" aria-label="Note types">
          <label class="agent-check">
            <input
              type="radio"
              name={`${p.id}-types`}
              checked={perms().types === null}
              onChange={() => setPerms({ types: null })}
            />{" "}
            All types
          </label>
          <label class="agent-check">
            <input
              type="radio"
              name={`${p.id}-types`}
              checked={perms().types !== null}
              onChange={() => setPerms({ types: [] })}
            />{" "}
            Only these types
          </label>
        </div>
        <Show when={perms().types !== null}>
          <div class="agent-type-list" role="group" aria-label="Allowed types">
            <For each={typeNames()}>
              {(t) => (
                <label class="agent-check">
                  <input
                    type="checkbox"
                    checked={perms().types!.includes(t.name)}
                    onChange={(e) => toggleType(t.name, e.currentTarget.checked)}
                  />{" "}
                  <TypeIcon name={t.icon} size={14} /> {t.label || t.name}
                </label>
              )}
            </For>
          </div>
          <Show when={perms().types!.length === 0}>
            <p class="muted">With no type ticked, the agent can read and change no notes.</p>
          </Show>
        </Show>
      </fieldset>
      <SaveBar dirty={p.dirty} save={p.save} />
    </section>
  );
}

function FileTab(p: AgentTabProps) {
  const [text, setText] = createSignal(p.agent.wcl);
  createEffect(on(() => p.agent.wcl, (w) => setText(w), { defer: true }));
  return (
    <section class="agent-tab" aria-label="File">
      <p class="muted">
        agents/{p.agent.name}.wcl, as docs/vault-format.md "Agent files" defines it. Saving checks it as the form does.
      </p>
      <Show when={p.dirty}>
        <p class="muted">The form has unsaved changes. Saving the file here replaces them.</p>
      </Show>
      <textarea
        class="ai-system-prompt agent-wcl"
        aria-label="Agent file"
        rows={24}
        spellcheck={false}
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
      />
      <SaveBar dirty={text() !== p.agent.wcl} save={() => p.saveWcl(text())} />
    </section>
  );
}

/** The agent screen. */
export function AgentScreen() {
  const params = useParams();
  const [search] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const pane = usePane();
  const uid = createUniqueId();
  // Settings > AI, for the shipped prompt, is the web app's; in the apps the agent itself is
  // read and saved on the home server, as in the web app.
  const [app] = createResource(appInfo);
  const [ai] = createResource(
    () => (app.state === "ready" && app() === null ? true : null),
    () => api.aiStatus().catch(() => null),
  );
  // The accounts are read in every client: an app forwards the route to the home server.
  // A renamed account rewrites the agents that name it (accounts A6), so read them again then.
  const [accounts] = createResource(() => [agentsVersion()], () => api.aiAccounts().catch(() => null));
  const [agent, setAgent] = createSignal<Agent | null>(null);
  const [draft, setDraftSignal] = createSignal<AgentForm | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  function show(a: Agent) {
    setAgent(a);
    setDraftSignal(agentForm(a));
  }

  // Read the agent, and again after the sidebar changes one or it changes elsewhere. A change
  // elsewhere leaves unsaved edits alone.
  createEffect(
    on([() => params.name, agentsVersion], ([name], prev) => {
      if (prev && prev[0] === name && dirty()) return;
      setError(null);
      api
        .agent(name!)
        .then((a) => {
          if (params.name === name) show(a);
        })
        .catch((e: unknown) => {
          setAgent(null);
          setError(e instanceof ApiError && e.status === 404 ? `There is no agent called ${name}.` : errorText(e));
        });
    }),
  );

  createEffect(() => {
    const a = agent();
    if (a) pane?.setTitle({ title: agentLabel(a), icon: a.icon || AGENT_ICON });
  });
  onCleanup(() => pane?.setTitle(null));

  const dirty = createMemo(() => {
    const a = agent();
    const d = draft();
    return !!a && !!d && JSON.stringify(agentForm(a)) !== JSON.stringify(d);
  });

  const view = createMemo(() => agentView(params.name!, params.section, params.tab, location.search));
  // An old address goes to its new place, in place of itself in the pane's history.
  createEffect(() => {
    const v = view();
    if ("redirect" in v) navigate(v.redirect, { replace: true });
  });
  const settingsTab = () => {
    const v = view();
    return "tab" in v ? v.tab : null;
  };
  const tab = () => AGENT_TABS.find((t) => t.id === settingsTab()) ?? AGENT_TABS[0]!;
  const addressThread = () => {
    const t = search.thread;
    return (Array.isArray(t) ? t[0] : t) || null;
  };
  // The chat shows once it has been asked for and then stays, hidden behind the settings, so
  // the session and what is typed in it are there on the way back.
  const [chatSeen, setChatSeen] = createSignal(false);
  const inChat = () => {
    const v = view();
    return "view" in v && v.view === "chat";
  };
  createEffect(() => {
    if (inChat()) setChatSeen(true);
  });
  /** The session the chat shows, for the way back from the settings. */
  const [shownThread, setShownThread] = createSignal<string | null>(null);
  createEffect(() => {
    if (inChat()) setShownThread(addressThread());
  });

  function onThread(id: string | null) {
    setShownThread(id);
    if (!inChat() || addressThread() === id) return;
    navigate(agentHref(params.name!, id), { replace: true });
  }

  async function put(body: AgentForm | { wcl: string }) {
    const name = params.name!;
    const saved = await api.putAgent(name, body);
    show(saved);
    void refreshAgents();
  }

  function onTabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const i = AGENT_TABS.indexOf(tab());
    const n = AGENT_TABS.length;
    const at = e.key === "Home" ? 0 : e.key === "End" ? n - 1 : (i + (e.key === "ArrowRight" ? 1 : n - 1)) % n;
    const next = AGENT_TABS[at]!;
    go(next.id);
    queueMicrotask(() => document.getElementById(`${uid}-tab-${next.id}`)?.focus());
  }

  function go(id: string) {
    navigate(agentSettingsHref(params.name!, id), { replace: true });
  }

  return (
    <section class="list-page agent-screen" classList={{ "agent-screen-chat": inChat() }}>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={agent() && draft() ? { a: agent()!, d: draft()! } : null}>
        {(v) => (
          <>
            <Show when={chatSeen()}>
              <div class="agent-chat-wrap" hidden={!inChat()}>
                <h1 class="visually-hidden">{agentLabel(v().a)}</h1>
                <AgentChat
                  agent={v().a.name}
                  label={agentLabel(v().a)}
                  thread={addressThread()}
                  onThread={onThread}
                  actions={
                    <>
                      <Show when={!v().a.enabled}>
                        <span class="agent-badge agent-badge-off">Disabled</span>
                      </Show>
                      <A
                        class="agent-settings-button icon-button"
                        href={agentSettingsHref(v().a.name)}
                        aria-label={`Settings of ${agentLabel(v().a)}`}
                        title="Agent settings"
                      >
                        <TypeIcon name="settings" size={18} />
                      </A>
                    </>
                  }
                />
              </div>
            </Show>
            <Show when={!inChat() && settingsTab()}>
              <div class="agent-settings">
                <header class="note-header agent-header">
                  <A class="agent-back icon-button" href={agentHref(v().a.name, shownThread())} aria-label="Back to the chat" title="Back to the chat">
                    <TypeIcon name="arrow-left" size={18} />
                  </A>
                  <h1>
                    <TypeIcon name={v().a.icon || AGENT_ICON} size={22} /> {agentLabel(v().a)}
                  </h1>
                  <Show when={v().a.built_in}>
                    <span class="agent-badge">Built in</span>
                  </Show>
                  <Show when={!v().a.enabled}>
                    <span class="agent-badge agent-badge-off">Disabled</span>
                  </Show>
                  <a class="agent-log-link" href={logHref(v().a.name)}>
                    <TypeIcon name="scroll-text" size={16} /> Show in log
                  </a>
                </header>
                <Show when={v().a.description}>
                  <p class="muted">{v().a.description}</p>
                </Show>
                <div class="agent-tabs" role="tablist" aria-label="Agent settings" onKeyDown={onTabKey}>
                  <For each={AGENT_TABS}>
                    {(t) => (
                      <button
                        type="button"
                        role="tab"
                        id={`${uid}-tab-${t.id}`}
                        aria-selected={tab().id === t.id}
                        aria-controls={`${uid}-panel`}
                        tabindex={tab().id === t.id ? 0 : -1}
                        onClick={() => go(t.id)}
                      >
                        {t.label}
                      </button>
                    )}
                  </For>
                </div>
                <div id={`${uid}-panel`} role="tabpanel" aria-labelledby={`${uid}-tab-${tab().id}`}>
                  {tab().render({
                    get agent() {
                      return v().a;
                    },
                    get draft() {
                      return v().d;
                    },
                    setDraft: (f) => setDraftSignal((d) => (d ? f(d) : d)),
                    get dirty() {
                      return dirty();
                    },
                    save: () => put(draft()!),
                    saveWcl: (wcl) => put({ wcl }),
                    get ai() {
                      return ai() ?? null;
                    },
                    get accounts() {
                      return accounts() ?? null;
                    },
                    id: uid,
                  })}
                </div>
              </div>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
