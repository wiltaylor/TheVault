// The left sidebar's Agents section (agents G3, G6): the list with Assistant first, + to create
// one from a name and open it, the ⋯ menu's Open, Disable or Enable, and Delete after a confirm,
// Assistant without Disable or Delete, disabled agents dimmed, and a read-only list in the apps.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, AgentSummary } from "../api";

const agentsList = vi.fn();
const agent = vi.fn();
const putAgent = vi.fn();
const deleteAgent = vi.fn();
const appInfo = vi.fn();
const navigatePane = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, agents: agentsList, agent, putAgent, deleteAgent } };
});
vi.mock("../inApp", () => ({ appInfo }));
vi.mock("../workspace/store", async (orig) => {
  const real = await orig<typeof import("../workspace/store")>();
  return { ...real, navigatePane };
});

const { AgentsNav } = await import("./AgentsNav");
const { setAgentsOpen, setCreatingAgent } = await import("../agents");

const ASSISTANT: AgentSummary = { name: "assistant", label: "Assistant", icon: "sparkles", enabled: true, triggers: 0, built_in: true };
const TRIAGE: AgentSummary = { name: "inbox_triage", label: "Inbox triage", icon: null, enabled: true, triggers: 1, built_in: false };

function full(s: AgentSummary): Agent {
  return {
    name: s.name,
    label: s.label,
    description: null,
    icon: s.icon ?? null,
    enabled: s.enabled ?? true,
    provider: null,
    model: null,
    effort: null,
    system_prompt: "Be brief.",
    built_in: s.built_in,
    permissions: {
      tools: null,
      types: null,
      confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
      web: { search: true, read: true, download: true },
    },
    triggers: [{ name: "nightly", kind: "schedule", schedule: "0 2 * * *", timezone: "UTC", prompt: "Tidy up." }],
    wcl: "",
  };
}

let list: AgentSummary[];

beforeEach(() => {
  list = [ASSISTANT, TRIAGE];
  agentsList.mockImplementation(async () => ({ agents: list, tools: ["search_notes"] }));
  appInfo.mockResolvedValue(null);
  localStorage.clear();
  setAgentsOpen(true);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  setCreatingAgent(false);
});

function nav() {
  return within(screen.getByRole("navigation", { name: "Agents" }));
}

async function shown() {
  render(() => <AgentsNav onNavigate={() => {}} />);
  await screen.findByRole("link", { name: "Inbox triage" });
}

describe("the sidebar's Agents section", () => {
  it("lists every agent by label, Assistant first, each opening its agent screen", async () => {
    await shown();
    const links = nav().getAllByRole("link");
    expect(links.map((l) => l.textContent)).toEqual(["Assistant", "Inbox triage"]);
    expect(links[1]!.getAttribute("href")).toBe("/agents/inbox_triage");
    expect(nav().getByRole("button", { name: "Agents" })).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(nav().getByRole("button", { name: "Agents" }));
    expect(nav().queryByRole("link", { name: "Assistant" })).toBeNull();
    expect(localStorage.getItem("thevault.sidebar.agents")).toBe("false");
  });

  it("creates an agent from a name with + and opens it", async () => {
    await shown();
    putAgent.mockImplementation(async (name: string) => {
      list = [...list, { ...TRIAGE, name, label: "Daily digest" }];
      return full(list.at(-1)!);
    });
    fireEvent.click(nav().getByRole("button", { name: "New agent" }));
    const box = nav().getByLabelText("New agent name");
    fireEvent.input(box, { target: { value: "Daily digest" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(putAgent).toHaveBeenCalledWith("daily_digest", { label: "Daily digest" }));
    await waitFor(() => expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/agents/daily_digest"));
    expect(await nav().findByRole("link", { name: "Daily digest" })).toBeTruthy();
  });

  it("refuses a name an agent already has", async () => {
    await shown();
    fireEvent.click(nav().getByRole("button", { name: "New agent" }));
    const box = nav().getByLabelText("New agent name");
    fireEvent.input(box, { target: { value: "Inbox Triage" } });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await nav().findByRole("alert")).toHaveTextContent("There is already an agent called inbox_triage.");
    expect(putAgent).not.toHaveBeenCalled();
  });

  it("opens an agent from its menu, and from a right-click", async () => {
    await shown();
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Open" }));
    expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/agents/inbox_triage");
    fireEvent.contextMenu(nav().getByRole("link", { name: "Inbox triage" }));
    expect(screen.getByRole("menu", { name: "Actions for Inbox triage" })).toBeTruthy();
  });

  it("disables an agent, keeping the rest of its file, and shows it dimmed", async () => {
    await shown();
    agent.mockResolvedValue(full(TRIAGE));
    putAgent.mockImplementation(async (_name: string, body: { enabled: boolean }) => {
      list = [ASSISTANT, { ...TRIAGE, enabled: body.enabled }];
      return { ...full(TRIAGE), enabled: body.enabled };
    });
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Disable" }));
    await waitFor(() => expect(putAgent).toHaveBeenCalled());
    const [name, body] = putAgent.mock.calls[0]!;
    expect(name).toBe("inbox_triage");
    expect(body).toMatchObject({ enabled: false, system_prompt: "Be brief.", triggers: [{ name: "nightly" }] });
    expect(body).not.toHaveProperty("wcl");
    await waitFor(() => expect(nav().getByRole("link", { name: /Inbox triage/ }).closest(".agent-row")).toHaveClass("agent-disabled"));
    expect(nav().getByRole("link", { name: /Inbox triage/ })).toHaveAttribute("title", "Inbox triage (disabled)");
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    expect(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Enable" })).toBeTruthy();
  });

  it("deletes an agent only after the confirm", async () => {
    await shown();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Delete" }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Delete the agent Inbox triage?"));
    expect(deleteAgent).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    deleteAgent.mockImplementation(async () => {
      list = [ASSISTANT];
    });
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(deleteAgent).toHaveBeenCalledWith("inbox_triage"));
    await waitFor(() => expect(nav().queryByRole("link", { name: "Inbox triage" })).toBeNull());
    confirm.mockRestore();
  });

  it("gives Assistant no Disable or Delete (agents G6)", async () => {
    await shown();
    fireEvent.click(nav().getByRole("button", { name: "Actions for Assistant" }));
    const menu = within(screen.getByRole("menu"));
    expect(menu.getAllByRole("menuitem").map((b) => b.textContent)).toEqual(["Open"]);
  });

  it("is read only in the apps: no + and only Open in the menu", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await shown();
    await waitFor(() => expect(nav().queryByRole("button", { name: "New agent" })).toBeNull());
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    expect(within(screen.getByRole("menu")).getAllByRole("menuitem").map((b) => b.textContent)).toEqual(["Open"]);
  });
});
