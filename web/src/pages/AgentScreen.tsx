// The agent screen, `/agents/<name>` (agents G5): one agent as tabs. Chat, the default, holds its
// threads (agents G7, `ai/AgentChat.tsx`). Settings holds its
// label, icon, model and system prompt; Permissions its own tools, confirmation and internet
// switches and note types (agents G4); File its WCL. Settings and Permissions edit one draft, so
// moving between them keeps unsaved changes, and either tab's Save writes the whole agent. In the
// apps the screen is read only: agent files need `admin`, which a device token never has (G2).

import { useNavigate, useParams } from "@solidjs/router";
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
} from "solid-js";
import {
  AGENT_ICON,
  AGENT_PROMPT_MAX,
  CONFIRM_SWITCHES,
  WEB_SWITCHES,
  agentForm,
  agentLabel,
  agentTools,
  agentsVersion,
  refreshAgents,
  toolGroups,
} from "../agents";
import {
  type Agent,
  type AgentEffort,
  type AgentForm,
  type AiModel,
  type AiProvider,
  type AiStatus,
  ApiError,
  api,
} from "../api";
import { AgentChat } from "../ai/AgentChat";
import { IconPicker } from "../components/IconPicker";
import { TypeIcon } from "../components/Icon";
import { Spinner } from "../components/SyncStatus";
import { appInfo } from "../inApp";
import { types } from "../session";
import { usePane } from "../workspace/pane";
import { modelLabel } from "./Ai";
import { errorText } from "./TypeEditor";

export const PROVIDERS: [AiProvider, string][] = [
  ["anthropic", "Anthropic"],
  ["openai", "OpenAI"],
  ["chatgpt", "ChatGPT"],
];

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
  /** In the apps: nothing can be changed here. */
  readOnly: boolean;
  /** Settings > AI as it stands, in the web app; null in the apps or before it loads. */
  ai: AiStatus | null;
  /** A prefix for element ids, unique to this screen. */
  id: string;
}

/**
 * The screen's tabs, in order; the first is the default. Tickets that add a tab (Chat, Triggers)
 * add it here.
 */
export const AGENT_TABS: { id: string; label: string; render: (p: AgentTabProps) => JSX.Element }[] = [
  { id: "chat", label: "Chat", render: (p) => <AgentChat agent={p.agent.name} label={agentLabel(p.agent)} /> },
  { id: "settings", label: "Settings", render: (p) => <SettingsTab {...p} /> },
  { id: "permissions", label: "Permissions", render: (p) => <PermissionsTab {...p} /> },
  { id: "file", label: "File", render: (p) => <FileTab {...p} /> },
];

/** A Save button with its spinner, and the result of the last save below it. */
function SaveBar(props: { dirty: boolean; readOnly: boolean; save: () => Promise<void>; label?: string }) {
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
 * The model: chosen from the provider's live list, with the provider's default first, or typed
 * when the list cannot be read (ai D6, D24). Empty means the provider's default model.
 */
export function ModelPicker(props: {
  id: string;
  provider: AiProvider | null;
  value: string | null;
  onChange: (model: string | null) => void;
  disabled: boolean;
  /** Whether to ask the provider for its models: not in the apps. */
  live: boolean;
}) {
  const [list] = createResource(
    () => (props.live && props.provider ? props.provider : null),
    async (p) => {
      try {
        const l = await api.aiModels(p);
        return { models: l.models, model: l.model, error: null as string | null };
      } catch (e) {
        return { models: [] as AiModel[], model: null, error: errorText(e) };
      }
    },
  );
  const typed = () => !props.live || !props.provider || !!list()?.error;
  const defaultLabel = () => {
    const m = list()?.model;
    return m ? `Provider default (${m})` : "Provider default";
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
              placeholder="Provider default"
              value={props.value ?? ""}
              disabled={props.disabled}
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
              disabled={props.disabled}
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
      <Show when={props.live && typed()}>
        <p class="muted agent-model-note">
          {props.provider
            ? `The model list could not be read${list()?.error ? ` (${list()!.error})` : ""}: type a model id, or leave it empty for the provider's default.`
            : "No default provider is set in Settings > AI: choose a provider, or type a model id."}
        </p>
      </Show>
    </>
  );
}

function SettingsTab(p: AgentTabProps) {
  const id = (what: string) => `${p.id}-${what}`;
  const set = (patch: Partial<AgentForm>) => p.setDraft((d) => ({ ...d, ...patch }));
  const defaultProvider = () => p.ai?.default_provider ?? null;
  const provider = () => p.draft.provider ?? defaultProvider();
  const providerLabel = (name: AiProvider | null) => PROVIDERS.find(([n]) => n === name)?.[1];
  const prompt = () => p.draft.system_prompt ?? "";
  const length = () => [...prompt()].length;
  return (
    <section class="agent-tab" aria-label="Settings">
      <fieldset class="agent-fieldset" disabled={p.readOnly}>
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
          <label for={id("provider")}>Provider</label>
          <select
            id={id("provider")}
            value={p.draft.provider ?? ""}
            onChange={(e) => set({ provider: (e.currentTarget.value || null) as AiProvider | null, model: null })}
          >
            <option value="">
              {defaultProvider() ? `Settings > AI default (${providerLabel(defaultProvider())})` : "Settings > AI default"}
            </option>
            <For each={PROVIDERS}>{([v, l]) => <option value={v}>{l}</option>}</For>
          </select>
          <label for={id("model")}>Model</label>
          <div>
            <ModelPicker
              id={id("model")}
              provider={provider()}
              value={p.draft.model}
              disabled={p.readOnly}
              live={!p.readOnly}
              onChange={(model) => set({ model })}
            />
          </div>
          <label for={id("effort")}>Effort</label>
          <select
            id={id("effort")}
            value={p.draft.effort ?? ""}
            onChange={(e) => set({ effort: (e.currentTarget.value || null) as AgentEffort | null })}
          >
            <option value="">Provider default</option>
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
          <Show when={!p.readOnly}>
            <button type="button" disabled={p.draft.system_prompt === null} onClick={() => set({ system_prompt: null })}>
              Use the shipped prompt
            </button>
          </Show>
        </div>
      </fieldset>
      <SaveBar dirty={p.dirty} readOnly={p.readOnly} save={p.save} />
    </section>
  );
}

function PermissionsTab(p: AgentTabProps) {
  const perms = () => p.draft.permissions;
  const setPerms = (patch: Partial<AgentForm["permissions"]>) =>
    p.setDraft((d) => ({ ...d, permissions: { ...d.permissions, ...patch } }));
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
  return (
    <section class="agent-tab" aria-label="Permissions">
      <p class="agent-perm-note">
        These permissions are {agentLabel(p.agent)}'s own: they replace Settings {">"} AI for this agent, and nothing is
        inherited. The person or token running a chat still limits what it can change.
      </p>
      <fieldset class="agent-fieldset" disabled={p.readOnly}>
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
                  checked={perms().confirm[s.key]}
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
      <SaveBar dirty={p.dirty} readOnly={p.readOnly} save={p.save} />
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
      <Show when={p.dirty && !p.readOnly}>
        <p class="muted">The form has unsaved changes. Saving the file here replaces them.</p>
      </Show>
      <textarea
        class="ai-system-prompt agent-wcl"
        aria-label="Agent file"
        rows={24}
        spellcheck={false}
        readOnly={p.readOnly}
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
      />
      <SaveBar dirty={text() !== p.agent.wcl} readOnly={p.readOnly} save={() => p.saveWcl(text())} />
    </section>
  );
}

/** The agent screen. */
export function AgentScreen() {
  const params = useParams();
  const navigate = useNavigate();
  const pane = usePane();
  const uid = createUniqueId();
  const [app] = createResource(appInfo);
  const readOnly = () => app() !== null && app() !== undefined;
  const [ai] = createResource(
    () => (app.state === "ready" && app() === null ? true : null),
    () => api.aiStatus().catch(() => null),
  );
  const [agent, setAgent] = createSignal<Agent | null>(null);
  const [draft, setDraftSignal] = createSignal<AgentForm | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  function show(a: Agent) {
    setAgent(a);
    setDraftSignal(agentForm(a));
  }

  // Read the agent, and again after the sidebar changes one.
  createEffect(
    on([() => params.name, agentsVersion], ([name]) => {
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
  const tab = () => AGENT_TABS.find((t) => t.id === params.tab) ?? AGENT_TABS[0]!;

  async function put(body: AgentForm | { wcl: string }) {
    const name = params.name!;
    const saved = await api.putAgent(name, body);
    show(saved);
    void refreshAgents();
  }

  function onTabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const i = AGENT_TABS.indexOf(tab());
    const next = AGENT_TABS[(i + (e.key === "ArrowRight" ? 1 : AGENT_TABS.length - 1)) % AGENT_TABS.length]!;
    go(next.id);
    queueMicrotask(() => document.getElementById(`${uid}-tab-${next.id}`)?.focus());
  }

  function go(id: string) {
    navigate(`/agents/${encodeURIComponent(params.name!)}/${id}`, { replace: true });
  }

  return (
    <section class="list-page agent-screen">
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={agent() && draft() ? { a: agent()!, d: draft()! } : null}>
        {(v) => (
          <>
            <header class="note-header agent-header">
              <h1>
                <TypeIcon name={v().a.icon || AGENT_ICON} size={22} /> {agentLabel(v().a)}
              </h1>
              <Show when={v().a.built_in}>
                <span class="agent-badge">Built in</span>
              </Show>
              <Show when={!v().a.enabled}>
                <span class="agent-badge agent-badge-off">Disabled</span>
              </Show>
            </header>
            <Show when={v().a.description}>
              <p class="muted">{v().a.description}</p>
            </Show>
            <Show when={readOnly()}>
              <p class="agent-readonly" role="note">
                Edit agents in the web app. This app can show them but not change them.
              </p>
            </Show>
            <div class="agent-tabs" role="tablist" aria-label="Agent" onKeyDown={onTabKey}>
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
                get readOnly() {
                  return readOnly();
                },
                get ai() {
                  return ai() ?? null;
                },
                id: uid,
              })}
            </div>
          </>
        )}
      </Show>
    </section>
  );
}
