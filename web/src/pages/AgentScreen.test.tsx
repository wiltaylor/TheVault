// The agent screen (agents G5): the chat as its main view with the settings cog, the settings
// screen with its tabs at /agents/<name>/settings, the old tab addresses leading there, the
// General tab (the live model list, the system prompt with the
// shipped text as its placeholder, one Save of the whole agent), the Permissions tab (the tool
// checklist, the confirmation and internet switches, the note types), the File tab (the WCL,
// saved with the form's checks), the Triggers tab (schedules with their next runs, note events,
// the last runs linking to their threads, the tools a run is offered), Assistant always enabled,
// the Memory tab (the memories with search, Add, Edit with one Save, Delete after asking, also
// in the apps), and a read-only screen in the apps.

import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, AiStatus } from "../api";

const agent = vi.fn();
const agentsList = vi.fn();
const putAgent = vi.fn();
const aiStatus = vi.fn();
const aiModels = vi.fn();
const types = vi.fn();
const taxonomies = vi.fn();
const appInfo = vi.fn();
const agentThreads = vi.fn();
const aiProviders = vi.fn();
const agentRuns = vi.fn();
const agentThread = vi.fn();
const cron = vi.fn();
const agentMemories = vi.fn();
const createAgentMemory = vi.fn();
const updateAgentMemory = vi.fn();
const deleteAgentMemory = vi.fn();
const skillsList = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      agent,
      agents: agentsList,
      putAgent,
      aiStatus,
      aiModels,
      types,
      taxonomies,
      agentThreads,
      aiProviders,
      agentRuns,
      agentThread,
      cron,
      agentMemories,
      createAgentMemory,
      updateAgentMemory,
      deleteAgentMemory,
      skills: skillsList,
    },
  };
});
vi.mock("../inApp", () => ({ appInfo }));

const { AgentScreen, BROWSER_NOTE, OUTBOX_WARNING } = await import("./AgentScreen");
const { ApiError } = await import("../api");
const { refreshAgents } = await import("../agents");
const { reloadDefinitions } = await import("../session");
const { MENTION_WARNING } = await import("./AgentTriggers");

const SHIPPED = "You are the assistant inside TheVault.";

const TOOLS = [
  "search_notes",
  "query_notes",
  "get_note",
  "read_asset",
  "find_assets",
  "backlinks",
  "list_types",
  "get_type",
  "type_format",
  "propose_type_change",
  "list_taxonomies",
  "get_taxonomy",
  "taxonomy_format",
  "propose_taxonomy_change",
  "propose_taxonomy_delete",
  "list_agents",
  "get_agent",
  "propose_agent_change",
  "propose_agent_delete",
  "list_agent_memories",
  "get_agent_memory",
  "propose_agent_memory_change",
  "propose_agent_memory_delete",
  "list_skills",
  "get_skill",
  "propose_skill_change",
  "propose_skill_file_change",
  "propose_skill_file_delete",
  "propose_skill_delete",
  "propose_create",
  "propose_archive",
  "propose_delete",
  "propose_edit",
  "remember",
  "recall",
  "update_memory",
  "forget",
  "use_skill",
  "read_skill_file",
  "draft_email",
  "draft_sms",
  "draft_event",
  "fetch_url",
  "download_asset",
  "web_search",
];

function triage(over: Partial<Agent> = {}): Agent {
  return {
    name: "inbox_triage",
    label: "Inbox triage",
    description: "Files new email.",
    icon: "bot",
    enabled: true,
    provider: null,
    model: null,
    effort: null,
    system_prompt: null,
    built_in: false,
    permissions: {
      tools: null,
      types: null,
      confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
      web: { search: true, read: true, download: true },
      outbox: { send_without_review: false },
    },
    triggers: [{ name: "nightly", kind: "schedule", schedule: "0 2 * * *", timezone: "UTC", prompt: "Tidy up." }],
    wcl: 'label = "Inbox triage"\n',
    ...over,
  };
}

function status(): AiStatus {
  return {
    secrets_key: true,
    default_provider: "anthropic",
    providers: {
      anthropic: { configured: true, model: "claude-opus-5" },
      openai: { configured: false, model: null },
      chatgpt: { configured: false, model: null, account: null },
    },
    system_prompt: SHIPPED,
    default_system_prompt: SHIPPED,
    system_prompt_custom: false,
    confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
    web: { search: true, read: true, download: true },
  };
}

beforeEach(async () => {
  agent.mockResolvedValue(triage());
  putAgent.mockImplementation(async (name: string, body: Partial<Agent> | { wcl: string }) =>
    "wcl" in body ? triage({ wcl: body.wcl as string }) : { ...triage(), ...body, name, wcl: "saved\n" },
  );
  aiStatus.mockResolvedValue(status());
  aiModels.mockImplementation(async (provider: string) => {
    if (provider !== "anthropic") throw new ApiError(400, { error: "not_configured", message: "OpenAI is not set up" });
    return {
      provider,
      model: "claude-opus-5",
      models: [
        { id: "claude-opus-5", display_name: "Claude Opus 5" },
        { id: "claude-sonnet-5", display_name: "Claude Sonnet 5" },
      ],
    };
  });
  agentsList.mockResolvedValue({ agents: [], tools: TOOLS });
  types.mockResolvedValue([
    { name: "email", label: "Email", plural: "Emails", icon: "mail" },
    { name: "task", label: "Task", plural: "Tasks", icon: "check" },
  ]);
  taxonomies.mockResolvedValue([]);
  appInfo.mockResolvedValue(null);
  agentThreads.mockResolvedValue({ threads: [] });
  aiProviders.mockResolvedValue({ default_provider: "anthropic", providers: status().providers, confirm: status().confirm });
  agentRuns.mockResolvedValue({
    tools: ["search_notes", "get_note", "propose_edit"],
    triggers: [
      {
        name: "nightly",
        kind: "schedule",
        runs: [
          { id: 2, thread: "01JRUN2", started: 1767578400000, ended: 1767578403000, outcome: "failed", message: "Anthropic is unavailable" },
          { id: 1, thread: "01JRUN1", started: 1767492000000, ended: 1767492002000, outcome: "succeeded", message: null },
        ],
      },
    ],
  });
  agentThread.mockResolvedValue({
    id: "01JRUN2",
    title: "nightly 2026-01-05 02:00",
    agent: "inbox_triage",
    version: "t1",
    pending: 0,
    last_activity: null,
    messages: [{ role: "user", content: [{ type: "text", text: "Tidy up." }] }],
    steps: {},
    proposals: [],
  });
  cron.mockImplementation(async (schedule: string, timezone: string) => ({
    schedule,
    timezone,
    text: "Every day at 02:00",
    next: [1767578400000, 1767664800000],
  }));
  agentMemories.mockImplementation(async (_name: string, query = "") => {
    const all = [TEA, GARDEN];
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const memories = all.filter((m) => words.every((w: string) => `${m.title}\n${m.text}`.toLowerCase().includes(w)));
    return { memories, held: all.length, limit: 500 };
  });
  createAgentMemory.mockImplementation(async (agent: string, title: string, text: string) => ({ ...TEA, id: "01JNEW", agent, title, text }));
  updateAgentMemory.mockImplementation(async (_agent: string, id: string, change: { title: string; text: string }) => ({ ...TEA, id, ...change }));
  deleteAgentMemory.mockResolvedValue({ deleted: true });
  skillsList.mockResolvedValue([skill("pdf-forms"), skill("brand"), skill("broken", false)]);
  await refreshAgents();
  await reloadDefinitions();
});

function skill(name: string, valid = true) {
  return {
    name,
    description: `The ${name} skill.`,
    license: null,
    compatibility: null,
    metadata: {},
    allowed_tools: null,
    files: 1,
    size: 100,
    valid,
    issues: valid ? [] : [{ at: "name", message: "does not match the folder", severity: "error" }],
  };
}

const TEA = {
  id: "01JTEA",
  agent: "inbox_triage",
  title: "Tea",
  text: "Earl Grey, no milk.",
  version: "v1",
  created: "2026-01-04T10:00:00Z",
  updated: "2026-01-05T10:00:00Z",
};
const GARDEN = { ...TEA, id: "01JGARDEN", title: "Garden", text: "Water on Sundays.", version: "v2", updated: "2026-01-03T10:00:00Z" };

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function open(path = "/agents/inbox_triage/settings") {
  window.history.replaceState({}, "", path);
  render(() => (
    <Router>
      <Route path={["/agents/:name", "/agents/:name/:section", "/agents/:name/:section/:tab"]} component={AgentScreen} />
    </Router>
  ));
  await screen.findByRole("heading", { name: /Inbox triage|Assistant/ });
}

async function tab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
  return within(await screen.findByRole("region", { name }));
}

/** The body of the one PUT the screen sent. */
function sent(): Record<string, unknown> {
  expect(putAgent).toHaveBeenCalledTimes(1);
  return putAgent.mock.calls[0]![1] as Record<string, unknown>;
}

describe("the agent screen", () => {
  it("opens on the chat, with the session picker, Archive, Delete and the settings cog in its bar", async () => {
    await open("/agents/inbox_triage");
    const chat = within(await screen.findByRole("region", { name: "Chat" }));
    expect(chat.getByRole("button", { name: "Chat session: New chat" })).toBeTruthy();
    expect(chat.getByRole("button", { name: "Archive chat" })).toBeDisabled();
    expect(chat.getByRole("button", { name: "Delete chat" })).toBeDisabled();
    expect(chat.getByRole("link", { name: "Settings of Inbox triage" })).toHaveAttribute("href", "/agents/inbox_triage/settings");
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(agentThreads).toHaveBeenCalledWith("inbox_triage");
    expect(agent).toHaveBeenCalledWith("inbox_triage");
  });

  it("opens the settings from the cog, with General, Permissions, Memory, Triggers and File tabs, and goes back to the chat", async () => {
    await open("/agents/inbox_triage");
    fireEvent.click(await screen.findByRole("link", { name: "Settings of Inbox triage" }));
    await waitFor(() => expect(window.location.pathname).toBe("/agents/inbox_triage/settings"));
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["General", "Permissions", "Memory", "Triggers", "File"]);
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("region", { name: "Chat" })).toBeNull();
    await tab("Permissions");
    expect(window.location.pathname).toBe("/agents/inbox_triage/settings/permissions");
    fireEvent.click(screen.getByRole("link", { name: "Back to the chat" }));
    await waitFor(() => expect(window.location.pathname).toBe("/agents/inbox_triage"));
    expect(await screen.findByRole("region", { name: "Chat" })).toBeTruthy();
  });

  it("moves between the settings tabs with the arrow keys", async () => {
    await open();
    fireEvent.keyDown(screen.getByRole("tab", { name: "General" }), { key: "ArrowRight" });
    await waitFor(() => expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true"));
    fireEvent.keyDown(screen.getByRole("tab", { name: "Permissions" }), { key: "End" });
    await waitFor(() => expect(screen.getByRole("tab", { name: "File" })).toHaveAttribute("aria-selected", "true"));
    expect(window.location.pathname).toBe("/agents/inbox_triage/settings/file");
  });

  it("links to the agent's AI activity in the Log with Show in log (activity Y5)", async () => {
    await open();
    const link = screen.getByRole("link", { name: "Show in log" });
    expect(link.getAttribute("href")).toBe("/log?category=ai&actor=agent:inbox_triage");
  });

  it("opens the settings tab the address names", async () => {
    await open("/agents/inbox_triage/settings/permissions");
    expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
  });

  it("leads the old tab addresses to their new places", async () => {
    await open("/agents/inbox_triage/permissions");
    await waitFor(() => expect(window.location.pathname).toBe("/agents/inbox_triage/settings/permissions"));
    expect(await screen.findByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
    cleanup();
    await open("/agents/inbox_triage/chat?thread=01JRUN2");
    await waitFor(() => expect(window.location.pathname + window.location.search).toBe("/agents/inbox_triage?thread=01JRUN2"));
    expect(await screen.findByRole("region", { name: "Chat" })).toBeTruthy();
    cleanup();
    await open("/agents/inbox_triage/settings/nothing");
    await waitFor(() => expect(window.location.pathname).toBe("/agents/inbox_triage/settings"));
    expect(await screen.findByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true");
  });

  it("keeps unsaved settings over a visit to the chat", async () => {
    await open();
    fireEvent.input(within(screen.getByRole("region", { name: "General" })).getByLabelText("Label"), {
      target: { value: "Renamed" },
    });
    fireEvent.click(screen.getByRole("link", { name: "Back to the chat" }));
    fireEvent.click(await screen.findByRole("link", { name: "Settings of Inbox triage" }));
    const s = within(await screen.findByRole("region", { name: "General" }));
    expect((s.getByLabelText("Label") as HTMLInputElement).value).toBe("Renamed");
  });

  it("says when there is no such agent", async () => {
    agent.mockRejectedValue(new ApiError(404, { error: "not_found" }));
    window.history.replaceState({}, "", "/agents/gone");
    render(() => (
      <Router>
        <Route path="/agents/:name" component={AgentScreen} />
      </Router>
    ));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is no agent called gone.");
  });
});

describe("the General tab", () => {
  it("picks a model from the live list and edits the prompt, then saves the whole agent once", async () => {
    await open();
    const s = within(screen.getByRole("region", { name: "General" }));
    const model = (await s.findByLabelText("Model")) as HTMLSelectElement;
    await waitFor(() => expect(model.tagName).toBe("SELECT"));
    expect(aiModels).toHaveBeenCalledWith("anthropic");
    expect([...model.options].map((o) => o.textContent)).toEqual([
      "Provider default (claude-opus-5)",
      "Claude Opus 5 (claude-opus-5)",
      "Claude Sonnet 5 (claude-sonnet-5)",
    ]);
    expect((s.getByLabelText("Provider") as HTMLSelectElement).options[0]!.textContent).toBe(
      "Settings > AI default (Anthropic)",
    );
    const prompt = s.getByLabelText("System prompt") as HTMLTextAreaElement;
    expect(prompt.value).toBe("");
    expect(prompt.placeholder).toBe(SHIPPED);

    const save = s.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    fireEvent.change(model, { target: { value: "claude-sonnet-5" } });
    fireEvent.input(prompt, { target: { value: "File email. Be brief." } });
    fireEvent.change(s.getByLabelText("Effort"), { target: { value: "high" } });
    fireEvent.input(s.getByLabelText("Label"), { target: { value: "Triage" } });
    expect(save).toBeEnabled();
    fireEvent.click(save);

    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect(putAgent.mock.calls[0]![0]).toBe("inbox_triage");
    expect(sent()).toEqual({
      label: "Triage",
      description: "Files new email.",
      icon: "bot",
      enabled: true,
      provider: null,
      model: "claude-sonnet-5",
      effort: "high",
      system_prompt: "File email. Be brief.",
      permissions: triage().permissions,
      triggers: triage().triggers,
    });
    expect(await s.findByRole("status")).toHaveTextContent("Saved.");
  });

  it("clears a custom prompt with Use the shipped prompt", async () => {
    agent.mockResolvedValue(triage({ system_prompt: "Be terse." }));
    await open();
    const s = within(screen.getByRole("region", { name: "General" }));
    expect((s.getByLabelText("System prompt") as HTMLTextAreaElement).value).toBe("Be terse.");
    fireEvent.click(s.getByRole("button", { name: "Use the shipped prompt" }));
    expect((s.getByLabelText("System prompt") as HTMLTextAreaElement).value).toBe("");
    expect(s.getByRole("button", { name: "Use the shipped prompt" })).toBeDisabled();
    fireEvent.click(s.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(sent().system_prompt).toBeNull());
  });

  it("types the model when the provider's list cannot be read, and a new provider clears the model", async () => {
    agent.mockResolvedValue(triage({ provider: "anthropic", model: "claude-opus-5" }));
    await open();
    const s = within(screen.getByRole("region", { name: "General" }));
    await waitFor(() => expect(s.getByLabelText("Model").tagName).toBe("SELECT"));
    fireEvent.change(s.getByLabelText("Provider"), { target: { value: "openai" } });
    await waitFor(() => expect(aiModels).toHaveBeenCalledWith("openai"));
    const typed = (await s.findByPlaceholderText("Provider default")) as HTMLInputElement;
    expect(typed.value).toBe("");
    expect(s.getByText(/The model list could not be read \(OpenAI is not set up\)/)).toBeTruthy();
    fireEvent.change(typed, { target: { value: " gpt-6 " } });
    fireEvent.click(s.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(sent()).toMatchObject({ provider: "openai", model: "gpt-6" }));
  });

  it("shows the server's issues when a save is refused", async () => {
    putAgent.mockRejectedValue(new ApiError(400, { error: "invalid", issues: [{ at: "label", message: "is blank" }] }));
    await open();
    const s = within(screen.getByRole("region", { name: "General" }));
    fireEvent.input(s.getByLabelText("Label"), { target: { value: " " } });
    fireEvent.click(s.getByRole("button", { name: "Save" }));
    expect(await s.findByRole("alert")).toHaveTextContent("label: is blank");
  });

  it("keeps Assistant enabled: its switch cannot be turned off", async () => {
    agent.mockResolvedValue(triage({ name: "assistant", label: "Assistant", built_in: true }));
    await open("/agents/assistant/settings");
    const s = within(screen.getByRole("region", { name: "General" }));
    const enabled = s.getByRole("switch");
    expect(enabled).toBeChecked();
    expect(enabled).toBeDisabled();
    expect(s.getByText("Assistant is built in and always enabled.")).toBeTruthy();
    expect(screen.getByText("Built in")).toBeTruthy();
  });

  it("keeps unsaved changes when moving between tabs", async () => {
    await open();
    fireEvent.input(within(screen.getByRole("region", { name: "General" })).getByLabelText("Label"), {
      target: { value: "Renamed" },
    });
    await tab("Permissions");
    const s = await tab("General");
    expect((s.getByLabelText("Label") as HTMLInputElement).value).toBe("Renamed");
  });
});

describe("the Permissions tab", () => {
  it("groups the tools, all allowed by default, and saves a narrowed list", async () => {
    await open();
    const p = await tab("Permissions");
    expect(p.getByText(/they replace Settings > AI for this agent/)).toBeTruthy();
    const groups = p.getAllByRole("group").map((g) => g.querySelector(":scope > legend")?.textContent).filter(Boolean);
    expect(groups).toEqual(["Notes (read)", "Notes (changes)", "Memory", "Skills", "Types", "Taxonomies", "Agents", "Skill management", "Outbox", "Assets", "Internet"]);
    const web = p.getByRole("checkbox", { name: /Search the web/ });
    expect(web).toBeChecked();
    fireEvent.click(web);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    const tools = (sent().permissions as { tools: string[] }).tools;
    expect(tools).toHaveLength(TOOLS.length - 1);
    expect(tools).not.toContain("web_search");
  });

  it("allows every tool again as no list once all are ticked", async () => {
    agent.mockResolvedValue(triage({ permissions: { ...triage().permissions, tools: TOOLS.filter((t) => t !== "get_note") } }));
    await open();
    const p = await tab("Permissions");
    const get = p.getByRole("checkbox", { name: /Read a note/ });
    expect(get).not.toBeChecked();
    fireEvent.click(get);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect((sent().permissions as { tools: unknown }).tools).toBeNull());
  });

  it("lists the memory tools in a Memory group and saves an agent without them", async () => {
    await open();
    const p = await tab("Permissions");
    const group = p.getAllByRole("group").find((g) => g.querySelector(":scope > legend")?.textContent === "Memory")!;
    const names = Array.from(group.querySelectorAll("label")).map((l) => l.textContent?.trim());
    expect(names).toEqual([
      "Remember things remember",
      "Recall its memories recall",
      "Change its memories update_memory",
      "Forget memories forget",
    ]);
    fireEvent.click(p.getByRole("checkbox", { name: /Remember things/ }));
    fireEvent.click(p.getByRole("checkbox", { name: /Forget memories/ }));
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    const tools = (sent().permissions as { tools: string[] }).tools;
    expect(tools).toContain("recall");
    expect(tools).not.toContain("remember");
    expect(tools).not.toContain("forget");
  });

  it("lists the agent tools in an Agents group and saves Ask before changing agents", async () => {
    await open();
    const p = await tab("Permissions");
    const group = p.getAllByRole("group").find((g) => g.querySelector(":scope > legend")?.textContent === "Agents")!;
    const names = Array.from(group.querySelectorAll("label")).map((l) => l.textContent?.trim());
    expect(names).toEqual([
      "List agents list_agents",
      "Read an agent get_agent",
      "Create and change agents propose_agent_change",
      "Delete agents propose_agent_delete",
      "List other agents' memories list_agent_memories",
      "Read another agent's memory get_agent_memory",
      "Add and change other agents' memories propose_agent_memory_change",
      "Forget other agents' memories propose_agent_memory_delete",
    ]);
    const ask = p.getByRole("switch", { name: "Ask before changing agents" });
    expect(ask).toBeChecked();
    fireEvent.click(ask);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect((sent().permissions as { confirm: Record<string, boolean> }).confirm.agents).toBe(false);
  });

  it("lists the skill management tools in their group and saves Ask before changing skills", async () => {
    await open();
    const p = await tab("Permissions");
    const group = p.getAllByRole("group").find((g) => g.querySelector(":scope > legend")?.textContent === "Skill management")!;
    const names = Array.from(group.querySelectorAll("label")).map((l) => l.textContent?.trim());
    expect(names).toEqual([
      "List skills list_skills",
      "Read a skill get_skill",
      "Create skills and change SKILL.md propose_skill_change",
      "Add and change skill files propose_skill_file_change",
      "Delete skill files propose_skill_file_delete",
      "Delete skills propose_skill_delete",
    ]);
    const ask = p.getByRole("switch", { name: "Ask before changing skills" });
    expect(ask).toBeChecked();
    fireEvent.click(ask);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    const confirm = (sent().permissions as { confirm: Record<string, boolean> }).confirm;
    expect(confirm.skills).toBe(false);
    expect(confirm.agents).toBe(true);
  });

  it("saves the confirmation and internet switches and a chosen list of types", async () => {
    await open();
    const p = await tab("Permissions");
    fireEvent.click(p.getByRole("switch", { name: "Ask before creating and editing notes" }));
    fireEvent.click(p.getByRole("switch", { name: /Let the agent read web pages/ }));
    expect(p.getByRole("radio", { name: "All types" })).toBeChecked();
    fireEvent.click(p.getByRole("radio", { name: "Only these types" }));
    expect(p.getByText("With no type ticked, the agent can read and change no notes.")).toBeTruthy();
    fireEvent.click(p.getByRole("checkbox", { name: /Task/ }));
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect(sent().permissions).toEqual({
      tools: null,
      types: ["task"],
      confirm: { notes: false, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
      web: { search: true, read: false, download: true },
      outbox: { send_without_review: false },
    });
  });

  it("lists the draft tools in an Outbox group and saves Send from the Outbox without review, with a warning", async () => {
    await open();
    const p = await tab("Permissions");
    const group = p.getAllByRole("group").find((g) => g.querySelector(":scope > legend")?.textContent === "Outbox")!;
    const names = Array.from(group.querySelectorAll("label")).map((l) => l.textContent?.trim());
    expect(names).toEqual(["Draft emails draft_email", "Draft SMS messages draft_sms", "Draft calendar invites draft_event"]);
    const bypass = p.getByRole("switch", { name: "Send from the Outbox without review" });
    expect(bypass).not.toBeChecked();
    expect(p.queryByText(OUTBOX_WARNING)).toBeNull();
    fireEvent.click(bypass);
    expect(p.getByText(OUTBOX_WARNING)).toBeTruthy();
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect((sent().permissions as { outbox: unknown }).outbox).toEqual({ send_without_review: true });
  });

  it("shows the switch on for an agent that sends without review", async () => {
    agent.mockResolvedValue(triage({ permissions: { ...triage().permissions, outbox: { send_without_review: true } } }));
    await open();
    const p = await tab("Permissions");
    expect(p.getByRole("switch", { name: "Send from the Outbox without review" })).toBeChecked();
    expect(p.getByText(OUTBOX_WARNING)).toBeTruthy();
  });

  it("offers all skills by default, and saves a chosen list from the vault's skills", async () => {
    await open();
    const p = await tab("Permissions");
    const group = p.getAllByRole("group").find((g) => g.querySelector(":scope > legend")?.textContent === "Skills")!;
    expect(Array.from(group.querySelectorAll("label")).map((l) => l.textContent?.trim())).toEqual([
      "Use skills use_skill",
      "Read skill files read_skill_file",
    ]);
    const radios = within(p.getByRole("radiogroup", { name: "Skills" }));
    expect(radios.getByRole("radio", { name: "All skills" })).toBeChecked();
    expect(p.queryByRole("group", { name: "Allowed skills" })).toBeNull();
    fireEvent.click(radios.getByRole("radio", { name: "Only these skills" }));
    const list = within(await p.findByRole("group", { name: "Allowed skills" }));
    expect(list.getAllByRole("checkbox").map((c) => c.parentElement?.textContent?.trim())).toEqual([
      "brand",
      "broken (has errors)",
      "pdf-forms",
    ]);
    expect(p.getByText("With no skill ticked, the agent is offered none.")).toBeTruthy();
    fireEvent.click(list.getByRole("checkbox", { name: "pdf-forms" }));
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect((sent().permissions as { skills?: string[] }).skills).toEqual(["pdf-forms"]);
  });

  it("shows an agent's skill list, a name the vault lacks too, and sends none once All skills is chosen", async () => {
    agent.mockResolvedValue(triage({ permissions: { ...triage().permissions, skills: ["brand", "gone"] } }));
    await open();
    const p = await tab("Permissions");
    const radios = within(p.getByRole("radiogroup", { name: "Skills" }));
    expect(radios.getByRole("radio", { name: "Only these skills" })).toBeChecked();
    const list = within(await p.findByRole("group", { name: "Allowed skills" }));
    await waitFor(() => expect(list.getAllByRole("checkbox")).toHaveLength(4));
    expect(list.getByRole("checkbox", { name: "brand" })).toBeChecked();
    expect(list.getByRole("checkbox", { name: "pdf-forms" })).not.toBeChecked();
    expect(list.getByRole("checkbox", { name: /gone/ })).toBeChecked();
    expect(p.getByText("(not in the vault)")).toBeTruthy();
    fireEvent.click(radios.getByRole("radio", { name: "All skills" }));
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect("skills" in (sent().permissions as object)).toBe(false);
  });

  it("shows Browser automation off with its warning line, and saves it on", async () => {
    await open();
    const p = await tab("Permissions");
    const sw = p.getByRole("switch", { name: "Browser automation" });
    expect(sw).not.toBeChecked();
    expect(p.getByText(BROWSER_NOTE)).toBeTruthy();
    // Not a tool in the tools list: an agent with every tool still has it off.
    expect(p.queryByText("browser_run")).toBeNull();
    fireEvent.click(sw);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect((sent().permissions as { browser?: boolean }).browser).toBe(true);
  });

  it("shows Browser automation on for an agent that has it, and sends nothing for it once off", async () => {
    agent.mockResolvedValue(triage({ permissions: { ...triage().permissions, browser: true } }));
    await open();
    const p = await tab("Permissions");
    const sw = p.getByRole("switch", { name: "Browser automation" });
    expect(sw).toBeChecked();
    fireEvent.click(sw);
    fireEvent.click(p.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect((sent().permissions as { browser?: boolean }).browser ?? false).toBe(false);
  });
});

describe("the Triggers tab", () => {
  it("shows a schedule with its next runs and its last runs, each linking to its thread", async () => {
    await open("/agents/inbox_triage/settings/triggers");
    const t = within(await screen.findByRole("region", { name: "Triggers" }));
    expect((t.getByLabelText("Schedule") as HTMLInputElement).value).toBe("0 2 * * *");
    expect(await t.findByText("Every day at 02:00")).toBeTruthy();
    expect(cron).toHaveBeenCalledWith("0 2 * * *", "UTC");
    expect(within(t.getByRole("list", { name: "Next runs" })).getAllByRole("listitem")).toHaveLength(2);
    const runs = within(await t.findByRole("list", { name: "Last runs" }));
    const items = runs.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Failed");
    expect(items[0]).toHaveTextContent("Anthropic is unavailable");
    expect(within(items[0]!).getByRole("link", { name: "Open thread" })).toHaveAttribute(
      "href",
      "/agents/inbox_triage?thread=01JRUN2",
    );
    expect(items[1]).toHaveTextContent("Succeeded");
    expect(t.getByText(/a run is offered only the tools that need no confirmation/).parentElement).toHaveTextContent(
      "Search notes, Read a note, Edit notes.",
    );
    expect(agentRuns).toHaveBeenCalledWith("inbox_triage");
  });

  it("adds a note event trigger and saves it with the agent", async () => {
    await open("/agents/inbox_triage/settings/triggers");
    const t = within(await screen.findByRole("region", { name: "Triggers" }));
    fireEvent.click(t.getByRole("button", { name: "Add a note event" }));
    const card = within(t.getByRole("group", { name: "Trigger on_change" }));
    fireEvent.click(card.getByRole("checkbox", { name: "Changed" }));
    fireEvent.click(card.getByRole("checkbox", { name: "Email" }));
    fireEvent.input(card.getByLabelText("Where"), { target: { value: "processed != true" } });
    fireEvent.input(card.getByLabelText("Prompt"), { target: { value: "File this email." } });
    fireEvent.change(card.getByLabelText("Name"), { target: { value: "new_email" } });
    fireEvent.click(t.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect(sent().triggers).toEqual([
      triage().triggers[0],
      { name: "new_email", kind: "event", on: ["create", "update"], type: ["email"], where: "processed != true", prompt: "File this email." },
    ]);
  });

  it("adds a mention trigger, warns that the agent cannot change the note, and saves it", async () => {
    await open("/agents/inbox_triage/settings/triggers");
    const t = within(await screen.findByRole("region", { name: "Triggers" }));
    fireEvent.click(t.getByRole("button", { name: "Add a mention" }));
    const card = within(t.getByRole("group", { name: "Trigger asked" }));
    expect(card.getByText("When @mentioned in a note")).toBeTruthy();
    // Notes confirmation is on, so a run could not change the note it is called from.
    expect(card.getByRole("note")).toHaveTextContent(MENTION_WARNING);
    fireEvent.click(card.getByRole("checkbox", { name: "Task" }));
    fireEvent.click(t.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    expect(sent().triggers).toEqual([triage().triggers[0], { name: "asked", kind: "mention", type: ["task"], prompt: "" }]);
  });

  it("shows no warning on a mention trigger of an agent that edits notes itself", async () => {
    const perms = triage().permissions;
    agent.mockResolvedValue(
      triage({
        permissions: { ...perms, confirm: { ...perms.confirm, notes: false } },
        triggers: [{ name: "asked", kind: "mention", type: [], prompt: "Be brief." }],
      }),
    );
    await open("/agents/inbox_triage/settings/triggers");
    const t = within(await screen.findByRole("region", { name: "Triggers" }));
    const card = within(t.getByRole("group", { name: "Trigger asked" }));
    expect((card.getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe("Be brief.");
    expect(card.queryByRole("note")).toBeNull();
    // Without the edit tool it cannot change the note either.
    cleanup();
    agent.mockResolvedValue(
      triage({
        permissions: { ...perms, tools: ["get_note"], confirm: { ...perms.confirm, notes: false } },
        triggers: [{ name: "asked", kind: "mention", type: [], prompt: "" }],
      }),
    );
    await open("/agents/inbox_triage/settings/triggers");
    const again = within(await screen.findByRole("region", { name: "Triggers" }));
    expect(within(again.getByRole("group", { name: "Trigger asked" })).getByRole("note")).toHaveTextContent(MENTION_WARNING);
  });

  it("removes a trigger", async () => {
    await open("/agents/inbox_triage/settings/triggers");
    const t = within(await screen.findByRole("region", { name: "Triggers" }));
    fireEvent.click(t.getByRole("button", { name: "Remove trigger" }));
    expect(t.getByText("No triggers yet.")).toBeTruthy();
    fireEvent.click(t.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(sent().triggers).toEqual([]));
  });

  it("opens a run's thread in the chat", async () => {
    await open("/agents/inbox_triage/chat?thread=01JRUN2");
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("inbox_triage", "01JRUN2"));
  });
});

describe("the File tab", () => {
  it("edits and saves the agent's WCL", async () => {
    await open();
    const f = await tab("File");
    const text = f.getByLabelText("Agent file") as HTMLTextAreaElement;
    expect(text.value).toBe('label = "Inbox triage"\n');
    fireEvent.input(text, { target: { value: 'label = "Triage"\n' } });
    fireEvent.click(f.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalledWith("inbox_triage", { wcl: 'label = "Triage"\n' }));
  });

  it("shows why a file is refused", async () => {
    putAgent.mockRejectedValue(
      new ApiError(400, { error: "invalid", issues: [{ at: "provider", message: "is not anthropic, openai or chatgpt" }] }),
    );
    await open();
    const f = await tab("File");
    fireEvent.input(f.getByLabelText("Agent file"), { target: { value: 'provider = "x"\n' } });
    fireEvent.click(f.getByRole("button", { name: "Save" }));
    expect(await f.findByRole("alert")).toHaveTextContent("provider: is not anthropic, openai or chatgpt");
  });
});

describe("the Memory tab", () => {
  it("lists the agent's memories newest first and searches them", async () => {
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    const items = await m.findAllByRole("listitem");
    expect(items.map((i) => i.querySelector("strong")?.textContent)).toEqual(["Tea", "Garden"]);
    expect(items[0]).toHaveTextContent("Earl Grey, no milk.");
    expect(m.getByText("2 of 500 memories")).toBeTruthy();
    fireEvent.input(m.getByRole("searchbox", { name: "Search memories" }), { target: { value: "sundays" } });
    await waitFor(() => expect(m.getAllByRole("listitem")).toHaveLength(1));
    expect(agentMemories).toHaveBeenLastCalledWith("inbox_triage", "sundays");
    expect(m.getByRole("listitem")).toHaveTextContent("Garden");
    fireEvent.input(m.getByRole("searchbox", { name: "Search memories" }), { target: { value: "coffee" } });
    expect(await m.findByText("No memory matches the search.")).toBeTruthy();
  });

  it("adds a memory with its title and text", async () => {
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    await m.findAllByRole("listitem");
    fireEvent.click(m.getByRole("button", { name: "Add memory" }));
    const form = within(m.getByRole("form", { name: "New memory" }));
    const save = form.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    fireEvent.input(form.getByLabelText("Title"), { target: { value: "Coffee" } });
    fireEvent.input(form.getByLabelText("Text"), { target: { value: "Flat white." } });
    fireEvent.click(save);
    await waitFor(() => expect(createAgentMemory).toHaveBeenCalledWith("inbox_triage", "Coffee", "Flat white."));
    expect(await m.findByText("Memory added.")).toBeTruthy();
    expect(m.queryByRole("form")).toBeNull();
    expect(agentMemories.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it("edits a memory's title and text with one Save at its version", async () => {
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    await m.findAllByRole("listitem");
    fireEvent.click(m.getByRole("button", { name: "Edit Tea" }));
    const form = within(m.getByRole("form", { name: "Edit memory" }));
    expect(form.getByLabelText("Title")).toHaveValue("Tea");
    expect(form.getByLabelText("Text")).toHaveValue("Earl Grey, no milk.");
    expect(form.getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.input(form.getByLabelText("Title"), { target: { value: "Tea order" } });
    fireEvent.input(form.getByLabelText("Text"), { target: { value: "Green tea." } });
    fireEvent.click(form.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(updateAgentMemory).toHaveBeenCalledTimes(1));
    expect(updateAgentMemory).toHaveBeenCalledWith("inbox_triage", "01JTEA", { title: "Tea order", text: "Green tea.", base_version: "v1" });
    expect(await m.findByText("Saved.")).toBeTruthy();
  });

  it("shows why a save is refused", async () => {
    updateAgentMemory.mockRejectedValue(new ApiError(409, { error: "conflict", message: "the note changed since version v1" }));
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    await m.findAllByRole("listitem");
    fireEvent.click(m.getByRole("button", { name: "Edit Tea" }));
    fireEvent.input(m.getByLabelText("Text"), { target: { value: "Green tea." } });
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    expect(await m.findByRole("alert")).toHaveTextContent("changed since");
    expect(m.getByRole("form", { name: "Edit memory" })).toBeTruthy();
  });

  it("deletes a memory to the trash after asking", async () => {
    const ask = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    await m.findAllByRole("listitem");
    fireEvent.click(m.getByRole("button", { name: "Delete Garden" }));
    expect(ask).toHaveBeenCalledWith('Delete the memory "Garden"? It moves to the trash, where you can restore it.');
    expect(deleteAgentMemory).not.toHaveBeenCalled();
    fireEvent.click(m.getByRole("button", { name: "Delete Garden" }));
    await waitFor(() => expect(deleteAgentMemory).toHaveBeenCalledWith("inbox_triage", "01JGARDEN"));
    expect(await m.findByText("Memory moved to the trash.")).toBeTruthy();
    ask.mockRestore();
  });

  it("works in the apps too", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await open("/agents/inbox_triage/settings/memory");
    const m = within(await screen.findByRole("region", { name: "Memory" }));
    await m.findAllByRole("listitem");
    fireEvent.click(m.getByRole("button", { name: "Edit Tea" }));
    fireEvent.input(m.getByLabelText("Text"), { target: { value: "Green tea." } });
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(updateAgentMemory).toHaveBeenCalledTimes(1));
  });
});

describe("in the apps", () => {
  it("shows the agent read only, with Edit agents in the web app", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await open();
    expect(await screen.findByRole("note")).toHaveTextContent("Edit agents in the web app.");
    const s = within(screen.getByRole("region", { name: "General" }));
    expect(s.getByLabelText("Label")).toBeDisabled();
    expect(s.getByLabelText("System prompt")).toBeDisabled();
    expect(s.queryByRole("button", { name: "Save" })).toBeNull();
    expect(aiModels).not.toHaveBeenCalled();
    expect(aiStatus).not.toHaveBeenCalled();
    const p = await tab("Permissions");
    expect(p.getByRole("checkbox", { name: /Search notes/ })).toBeDisabled();
    expect(p.queryByRole("button", { name: "Save" })).toBeNull();
    const f = await tab("File");
    expect(f.getByLabelText("Agent file")).toHaveAttribute("readonly");
    expect(f.queryByRole("button", { name: "Save" })).toBeNull();
  });
});
