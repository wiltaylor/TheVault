// The agent screen (agents G5): the Settings tab (the live model list, the system prompt with the
// shipped text as its placeholder, one Save of the whole agent), the Permissions tab (the tool
// checklist, the confirmation and internet switches, the note types), the File tab (the WCL,
// saved with the form's checks), Assistant always enabled, and a read-only screen in the apps.

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
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, agent, agents: agentsList, putAgent, aiStatus, aiModels, types, taxonomies } };
});
vi.mock("../inApp", () => ({ appInfo }));

const { AgentScreen } = await import("./AgentScreen");
const { ApiError } = await import("../api");
const { refreshAgents } = await import("../agents");
const { reloadDefinitions } = await import("../session");

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
  "propose_create",
  "propose_archive",
  "propose_delete",
  "propose_edit",
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
      confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
      web: { search: true, read: true, download: true },
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
    confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
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
  await refreshAgents();
  await reloadDefinitions();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function open(path = "/agents/inbox_triage") {
  window.history.replaceState({}, "", path);
  render(() => (
    <Router>
      <Route path={["/agents/:name", "/agents/:name/:tab"]} component={AgentScreen} />
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
  it("has Settings, Permissions and File tabs, Settings first", async () => {
    await open();
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Settings", "Permissions", "File"]);
    expect(screen.getByRole("tab", { name: "Settings" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("region", { name: "Settings" })).toBeTruthy();
    expect(agent).toHaveBeenCalledWith("inbox_triage");
  });

  it("opens the tab the address names", async () => {
    await open("/agents/inbox_triage/permissions");
    expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
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

describe("the Settings tab", () => {
  it("picks a model from the live list and edits the prompt, then saves the whole agent once", async () => {
    await open();
    const s = within(screen.getByRole("region", { name: "Settings" }));
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
    const s = within(screen.getByRole("region", { name: "Settings" }));
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
    const s = within(screen.getByRole("region", { name: "Settings" }));
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
    const s = within(screen.getByRole("region", { name: "Settings" }));
    fireEvent.input(s.getByLabelText("Label"), { target: { value: " " } });
    fireEvent.click(s.getByRole("button", { name: "Save" }));
    expect(await s.findByRole("alert")).toHaveTextContent("label: is blank");
  });

  it("keeps Assistant enabled: its switch cannot be turned off", async () => {
    agent.mockResolvedValue(triage({ name: "assistant", label: "Assistant", built_in: true }));
    await open("/agents/assistant");
    const s = within(screen.getByRole("region", { name: "Settings" }));
    const enabled = s.getByRole("switch");
    expect(enabled).toBeChecked();
    expect(enabled).toBeDisabled();
    expect(s.getByText("Assistant is built in and always enabled.")).toBeTruthy();
    expect(screen.getByText("Built in")).toBeTruthy();
  });

  it("keeps unsaved changes when moving between tabs", async () => {
    await open();
    fireEvent.input(within(screen.getByRole("region", { name: "Settings" })).getByLabelText("Label"), {
      target: { value: "Renamed" },
    });
    await tab("Permissions");
    const s = await tab("Settings");
    expect((s.getByLabelText("Label") as HTMLInputElement).value).toBe("Renamed");
  });
});

describe("the Permissions tab", () => {
  it("groups the tools, all allowed by default, and saves a narrowed list", async () => {
    await open();
    const p = await tab("Permissions");
    expect(p.getByText(/they replace Settings > AI for this agent/)).toBeTruthy();
    const groups = p.getAllByRole("group").map((g) => g.querySelector(":scope > legend")?.textContent).filter(Boolean);
    expect(groups).toEqual(["Notes (read)", "Notes (changes)", "Types", "Taxonomies", "Assets", "Internet"]);
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
      confirm: { notes: false, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
      web: { search: true, read: false, download: true },
    });
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

describe("in the apps", () => {
  it("shows the agent read only, with Edit agents in the web app", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await open();
    expect(await screen.findByRole("note")).toHaveTextContent("Edit agents in the web app.");
    const s = within(screen.getByRole("region", { name: "Settings" }));
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
