// The left sidebar's Agents section (agents G3, G6): the list with Assistant first, + to create
// one from a name and open it, the ⋯ menu's Open, Disable or Enable, and Delete after a confirm,
// Assistant without Disable or Delete, disabled agents dimmed, and the same list in the apps.
// After the agents, one Skills item (skills S4): the Skills page, the number of skills and a
// warning mark when any is invalid, with no entry per skill.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, AgentSummary, SkillSummary } from "../api";

const agentsList = vi.fn();
const agent = vi.fn();
const putAgent = vi.fn();
const deleteAgent = vi.fn();
const appInfo = vi.fn();
const skillsList = vi.fn();
const navigatePane = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, agents: agentsList, agent, putAgent, deleteAgent, skills: skillsList } };
});
vi.mock("../inApp", () => ({ appInfo }));
vi.mock("../workspace/store", async (orig) => {
  const real = await orig<typeof import("../workspace/store")>();
  return { ...real, navigatePane };
});

const { AgentsNav } = await import("./AgentsNav");
const { setAgentsOpen, setCreatingAgent } = await import("../agents");
const { ApiError } = await import("../api");

function skill(name: string, more: Partial<SkillSummary> = {}): SkillSummary {
  return {
    name,
    description: `What ${name} does.`,
    license: null,
    compatibility: null,
    metadata: null,
    allowed_tools: null,
    files: 1,
    size: 120,
    valid: true,
    issues: [],
    ...more,
  };
}

let skillList: SkillSummary[];

const ASSISTANT: AgentSummary = { name: "assistant", label: "Assistant", icon: "sparkles", enabled: true, triggers: 0, built_in: true };
const TRIAGE: AgentSummary = { name: "inbox_triage", label: "Inbox triage", icon: null, enabled: true, triggers: 1, built_in: false };

function full(s: AgentSummary): Agent {
  return {
    name: s.name,
    label: s.label,
    description: null,
    icon: s.icon ?? null,
    enabled: s.enabled ?? true,
    account: null,
    model: null,
    effort: null,
    system_prompt: "Be brief.",
    built_in: s.built_in,
    permissions: {
      tools: null,
      types: null,
      confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
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
  skillList = [];
  skillsList.mockImplementation(async () => skillList);
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
    expect(links.map((l) => l.textContent)).toEqual(["Assistant", "Inbox triage", "Skills"]);
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

  it("creates, disables and deletes agents in the apps as in the web app", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await shown();
    expect(nav().getByRole("button", { name: "New agent" })).toBeTruthy();
    fireEvent.click(nav().getByRole("button", { name: "Actions for Inbox triage" }));
    expect(within(screen.getByRole("menu")).getAllByRole("menuitem").map((b) => b.textContent)).toEqual(["Open", "Disable", "Delete"]);
  });
});

describe("running agents (agents G17)", () => {
  it("shows a spinner, read as Running, on an agent while a turn runs on one of its threads", async () => {
    list = [ASSISTANT, { ...TRIAGE, running: 1 }];
    render(() => <AgentsNav onNavigate={() => {}} />);
    const triage = () => screen.getByRole("link", { name: /^Inbox triage/ });
    await waitFor(() => expect(within(triage()).getByRole("img", { name: "Running" })).toBeTruthy());
    expect(within(nav().getByRole("link", { name: "Assistant" })).queryByRole("img", { name: "Running" })).toBeNull();
  });
});

describe("the Skills item (skills S4)", () => {
  function skillsLink() {
    return nav().getByRole("link", { name: /^Skills/ });
  }

  it("is one item after the agents, opening the Skills page, with the number of skills", async () => {
    skillList = [skill("pdf-forms"), skill("notes-helper")];
    await shown();
    await waitFor(() => expect(skillsLink()).toHaveTextContent("2"));
    const links = nav().getAllByRole("link");
    expect(links.map((l) => l.getAttribute("href"))).toEqual(["/agents/assistant", "/agents/inbox_triage", "/skills"]);
    expect(skillsLink()).toHaveAttribute("title", "Skills: 2 skills");
    expect(skillsLink().closest(".agent-row")).toBeTruthy();
    expect(within(skillsLink()).queryByRole("img", { name: /invalid/ })).toBeNull();
    // No entry per skill, and no + menu for skills.
    expect(nav().queryByRole("link", { name: "pdf-forms" })).toBeNull();
    expect(nav().queryByRole("button", { name: "Add skill" })).toBeNull();
    expect(screen.queryByLabelText("Skill zip")).toBeNull();
  });

  it("shows a warning mark when any skill is invalid", async () => {
    skillList = [
      skill("pdf-forms"),
      skill("broken", { valid: false, issues: [{ at: "name", message: "name must match the folder", severity: "error" }] }),
    ];
    await shown();
    await waitFor(() => expect(within(skillsLink()).getByRole("img", { name: "1 invalid" })).toBeTruthy());
    expect(skillsLink()).toHaveAttribute("title", "Skills: 2 skills, 1 invalid");
  });

  it("shows no count from a server without skills (404), and no error", async () => {
    skillsList.mockRejectedValue(new ApiError(404, { error: "not_found" }));
    await shown();
    await waitFor(() => expect(skillsList).toHaveBeenCalled());
    expect(skillsLink().textContent).toBe("Skills");
    expect(nav().queryByRole("alert")).toBeNull();
  });

  it("is there in the apps too", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    skillList = [skill("pdf-forms")];
    await shown();
    await waitFor(() => expect(skillsLink()).toHaveTextContent("1"));
  });
});
