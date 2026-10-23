// Agents (agents G1 to G6): the list the left sidebar and the command palette show, the changes
// both make (create, enable or disable, delete), and the vocabulary the agent screen edits.

import { createRoot, createSignal } from "solid-js";
import { type Agent, type AgentForm, type AgentSummary, type AiConfirm, type AiWeb, api } from "./api";
import { appInfo } from "./inApp";

/** The built-in agent the chat panel uses by default (agents G6). */
export const ASSISTANT = "assistant";

/** The icon an agent without one shows. */
export const AGENT_ICON = "bot";

/** The longest system prompt, in characters, as the server holds it (agents G1). */
export const AGENT_PROMPT_MAX = 32_000;

export const AGENTS_OPEN_KEY = "thevault.sidebar.agents";

function loadOpen(): boolean {
  try {
    return localStorage.getItem(AGENTS_OPEN_KEY) !== "false";
  } catch {
    return true;
  }
}

const store = createRoot(() => {
  const [open, setOpen] = createSignal(loadOpen());
  const [list, setList] = createSignal<AgentSummary[]>([]);
  const [tools, setTools] = createSignal<string[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [readOnly, setReadOnly] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [version, setVersion] = createSignal(0);
  return { open, setOpen, list, setList, tools, setTools, error, setError, readOnly, setReadOnly, creating, setCreating, version, setVersion };
});

/** Whether the sidebar's Agents section is open. It starts open and is remembered per browser. */
export const agentsOpen = store.open;

export function setAgentsOpen(open: boolean) {
  store.setOpen(open);
  try {
    localStorage.setItem(AGENTS_OPEN_KEY, String(open));
  } catch {
    // Private browsing: the section stays as it is until the page closes.
  }
}

/** Every agent, Assistant first, as the server last listed them. */
export const agents = store.list;

/** Every tool an agent may be allowed, as the server names them. */
export const agentTools = store.tools;

/** Why the list could not be read, if it could not. */
export const agentsError = store.error;

/** In the apps, agents are read only: changing one needs the web app (agents G2). */
export const agentsReadOnly = store.readOnly;

/** Whether the sidebar shows the new agent's name box. */
export const creatingAgent = store.creating;
export const setCreatingAgent = store.setCreating;

/** Bumped by each change made here, so an open agent screen reads its agent again. */
export const agentsVersion = store.version;

/** How often the list reads again while a turn runs on an agent's thread (agents G17). */
export const AGENTS_RUNNING_POLL_MS = 5_000;

let runningPoll: ReturnType<typeof setTimeout> | undefined;

/**
 * Read the agent list again. While a turn runs on any agent's thread it reads again every
 * [`AGENTS_RUNNING_POLL_MS`], so the sidebar's spinners end without the live stream, as in the
 * apps.
 */
export async function refreshAgents(): Promise<void> {
  store.setReadOnly((await appInfo()) !== null);
  try {
    const l = await api.agents();
    store.setList(l.agents);
    store.setTools(l.tools ?? []);
    store.setError(null);
    clearTimeout(runningPoll);
    runningPoll = l.agents.some((a) => (a.running ?? 0) > 0) ? setTimeout(() => void refreshAgents(), AGENTS_RUNNING_POLL_MS) : undefined;
  } catch (e) {
    store.setError(e instanceof Error ? e.message : String(e));
  }
}

/** After a change: read the list again and tell open agent screens. */
export async function agentsChanged(): Promise<void> {
  store.setVersion(store.version() + 1);
  await refreshAgents();
}

/**
 * The agent name for what was typed in the new agent box: lower case, runs of anything but
 * a-z and 0-9 as one `_`, at most 64 characters, starting with a letter. Empty when nothing is
 * left.
 */
export function agentNameFrom(text: string): string {
  let name = text
    .trim()
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  if (name && !/^[a-z]/.test(name)) name = `agent_${name}`;
  return name.slice(0, 64).replace(/_+$/, "");
}

/** The type of the conversations with agents, which a trigger with no `type` is not called from. */
export const AGENT_THREAD_TYPE = "agent_thread";

/**
 * The agents a note of `type` may call by writing `@<name>` (agents G16), as the mention picker
 * offers them: each enabled agent with a mention trigger for the type, whose name or label
 * starts with `query`, ignoring case. With no type known, only agents every type may call.
 */
export function callableAgents(list: readonly AgentSummary[], type: string | undefined, query: string): AgentSummary[] {
  const q = query.trim().toLowerCase();
  return list.filter((a) => {
    const m = a.mention;
    if (!m || a.enabled === false || a.error) return false;
    const allowed = m.types === null ? type !== AGENT_THREAD_TYPE : type !== undefined && m.types.includes(type);
    if (!allowed) return false;
    return q === "" || a.name.toLowerCase().startsWith(q) || agentLabel(a).toLowerCase().startsWith(q);
  });
}

/** What an agent is called: its label, or its name. */
export function agentLabel(a: Pick<AgentSummary, "name" | "label">): string {
  return a.label?.trim() || a.name;
}

/** The form `PUT /api/agents/{name}` takes for `a`: the parsed agent without what GET adds. */
export function agentForm(a: Agent): AgentForm {
  return {
    label: a.label,
    description: a.description ?? null,
    icon: a.icon ?? null,
    enabled: a.enabled,
    provider: a.provider ?? null,
    model: a.model ?? null,
    effort: a.effort ?? null,
    system_prompt: a.system_prompt ?? null,
    permissions: {
      tools: a.permissions.tools ?? null,
      types: a.permissions.types ?? null,
      confirm: { ...a.permissions.confirm },
      web: { ...a.permissions.web },
    },
    triggers: a.triggers.map((t) => ({ ...t })),
  };
}

/**
 * Create the agent for what was typed: its name from `agentNameFrom`, its label the text as
 * typed. Refuses a name an agent already has. Answers the new agent's name.
 */
export async function createAgent(text: string): Promise<string> {
  const name = agentNameFrom(text);
  if (!name) throw new Error("Type a name with at least one letter or digit.");
  if (agents().some((a) => a.name === name)) throw new Error(`There is already an agent called ${name}.`);
  await api.putAgent(name, { label: text.trim() || name });
  await agentsChanged();
  return name;
}

/** Turn an agent's `enabled` on or off, keeping the rest of its file. */
export async function setAgentEnabled(name: string, enabled: boolean): Promise<void> {
  const a = await api.agent(name);
  await api.putAgent(name, { ...agentForm(a), enabled });
  await agentsChanged();
}

/** Delete an agent's file. Its threads stay. */
export async function deleteAgent(name: string): Promise<void> {
  await api.deleteAgent(name);
  await agentsChanged();
}

/** The tool checklist's groups, in order (agents G5). A tool the server adds later lands in Other. */
export const TOOL_GROUPS: { group: string; tools: [string, string][] }[] = [
  {
    group: "Notes (read)",
    tools: [
      ["search_notes", "Search notes"],
      ["query_notes", "Query notes"],
      ["get_note", "Read a note"],
      ["backlinks", "List backlinks"],
    ],
  },
  {
    group: "Notes (changes)",
    tools: [
      ["propose_edit", "Edit notes"],
      ["propose_create", "Create notes"],
      ["propose_archive", "Archive notes"],
      ["propose_delete", "Move notes to the trash"],
    ],
  },
  {
    group: "Types",
    tools: [
      ["list_types", "List types"],
      ["get_type", "Read a type"],
      ["type_format", "Read the type file format"],
      ["propose_type_change", "Change types"],
    ],
  },
  {
    group: "Taxonomies",
    tools: [
      ["list_taxonomies", "List taxonomies"],
      ["get_taxonomy", "Read a taxonomy"],
      ["taxonomy_format", "Read the taxonomy file format"],
      ["propose_taxonomy_change", "Change taxonomies"],
      ["propose_taxonomy_delete", "Delete taxonomies"],
    ],
  },
  {
    group: "Assets",
    tools: [
      ["find_assets", "Find images and files"],
      ["read_asset", "Read an image or file"],
    ],
  },
  {
    group: "Internet",
    tools: [
      ["web_search", "Search the web"],
      ["fetch_url", "Read web pages"],
      ["download_asset", "Download files into the vault"],
    ],
  },
];

/** `tools`, the server's tool names, in the checklist's groups, each tool with its label. */
export function toolGroups(tools: readonly string[]): { group: string; tools: { name: string; label: string }[] }[] {
  const known = new Set(TOOL_GROUPS.flatMap((g) => g.tools.map(([n]) => n)));
  const out = TOOL_GROUPS.map((g) => ({
    group: g.group,
    tools: g.tools.filter(([n]) => tools.includes(n)).map(([name, label]) => ({ name, label })),
  }));
  const other = tools.filter((t) => !known.has(t)).map((name) => ({ name, label: name }));
  if (other.length) out.push({ group: "Other", tools: other });
  return out.filter((g) => g.tools.length > 0);
}

/** The confirmation switches, in order, with their labels (ai D22). */
export const CONFIRM_SWITCHES: { key: keyof AiConfirm; label: string }[] = [
  { key: "notes", label: "Ask before creating and editing notes" },
  { key: "archive", label: "Ask before archiving notes" },
  { key: "delete", label: "Ask before moving notes to the trash" },
  { key: "types", label: "Ask before changing types" },
  { key: "downloads", label: "Ask before downloading files into the vault" },
  { key: "taxonomies", label: "Ask before changing taxonomies" },
];

/** The internet switches, in order, with their labels (ai D28). */
export const WEB_SWITCHES: { key: keyof AiWeb; label: string; note?: string }[] = [
  { key: "search", label: "Let the agent search the web", note: "Needs a Brave Search key in Settings > Integrations." },
  { key: "read", label: "Let the agent read web pages" },
  { key: "download", label: "Let the agent download files into the vault" },
];
