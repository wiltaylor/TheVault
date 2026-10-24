// The left sidebar's Agents section (agents G3, G6): the list with Assistant first, + to create
// one from a name and open it, the ⋯ menu's Open, Disable or Enable, and Delete after a confirm,
// Assistant without Disable or Delete, disabled agents dimmed, and a read-only list in the apps.
// Beneath the agents, the Skills subsection (skills S4): each skill by name with a warning mark
// when invalid, the + menu's New skill and Import from a zip or a folder, read only in the apps.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, AgentSummary, SkillSummary } from "../api";

const agentsList = vi.fn();
const agent = vi.fn();
const putAgent = vi.fn();
const deleteAgent = vi.fn();
const appInfo = vi.fn();
const skillsList = vi.fn();
const putSkill = vi.fn();
const importSkill = vi.fn();
const navigatePane = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, agents: agentsList, agent, putAgent, deleteAgent, skills: skillsList, putSkill, importSkill } };
});
vi.mock("../inApp", () => ({ appInfo }));
vi.mock("../workspace/store", async (orig) => {
  const real = await orig<typeof import("../workspace/store")>();
  return { ...real, navigatePane };
});

const { AgentsNav } = await import("./AgentsNav");
const { setAgentsOpen, setCreatingAgent } = await import("../agents");
const { setCreatingSkill, DESCRIPTION_PLACEHOLDER } = await import("../skills");
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
    provider: null,
    model: null,
    effort: null,
    system_prompt: "Be brief.",
    built_in: s.built_in,
    permissions: {
      tools: null,
      types: null,
      confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true },
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
  setCreatingSkill(false);
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

describe("running agents (agents G17)", () => {
  it("shows a spinner, read as Running, on an agent while a turn runs on one of its threads", async () => {
    list = [ASSISTANT, { ...TRIAGE, running: 1 }];
    render(() => <AgentsNav onNavigate={() => {}} />);
    const triage = () => screen.getByRole("link", { name: /^Inbox triage/ });
    await waitFor(() => expect(within(triage()).getByRole("img", { name: "Running" })).toBeTruthy());
    expect(within(nav().getByRole("link", { name: "Assistant" })).queryByRole("img", { name: "Running" })).toBeNull();
  });
});

describe("the Skills subsection (skills S4)", () => {
  function skillsGroup() {
    return within(screen.getByRole("group", { name: "Skills" }));
  }

  it("lists every skill by name beneath the agents, an invalid one with a warning mark", async () => {
    skillList = [
      skill("pdf-forms"),
      skill("broken", { valid: false, issues: [{ at: "name", message: "name must match the folder", severity: "error" }] }),
    ];
    render(() => <AgentsNav onNavigate={() => {}} />);
    await waitFor(() => expect(skillsGroup().getAllByRole("link")).toHaveLength(2));
    const links = skillsGroup().getAllByRole("link");
    // Sorted by name.
    expect(links.map((l) => l.textContent)).toEqual(["broken (invalid)", "pdf-forms"]);
    expect(links[1]!.getAttribute("href")).toBe("/skills/pdf-forms");
    expect(links[1]!).toHaveAttribute("title", "pdf-forms: What pdf-forms does.");
    expect(links[0]!).toHaveAttribute("title", "broken is invalid: name must match the folder");
    expect(links[0]!.closest(".agent-row")).toHaveClass("agent-broken");
    // Beneath the agents, inside the Agents section.
    const agentLinks = nav().getAllByRole("link").map((l) => l.textContent);
    expect(agentLinks.indexOf("Inbox triage")).toBeLessThan(agentLinks.indexOf("pdf-forms"));
  });

  it("lists no skills from a server that has none (404)", async () => {
    skillsList.mockRejectedValue(new ApiError(404, { error: "not_found" }));
    render(() => <AgentsNav onNavigate={() => {}} />);
    await screen.findByRole("link", { name: "Inbox triage" });
    expect(await skillsGroup().findByText("No skills yet.")).toBeTruthy();
    expect(nav().queryByRole("alert")).toBeNull();
  });

  it("makes a new skill from a name, with the placeholder description, and opens it", async () => {
    await shown();
    putSkill.mockImplementation(async (name: string) => {
      skillList = [skill(name, { description: DESCRIPTION_PLACEHOLDER })];
    });
    fireEvent.click(skillsGroup().getByRole("button", { name: "Add skill" }));
    const menu = within(screen.getByRole("menu", { name: "Add skill" }));
    expect(menu.getAllByRole("menuitem").map((b) => b.textContent)).toContain("Import skill from a zip");
    fireEvent.click(menu.getByRole("menuitem", { name: "New skill" }));
    const box = skillsGroup().getByLabelText("New skill name");
    fireEvent.input(box, { target: { value: "PDF Forms" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(putSkill).toHaveBeenCalled());
    const [name, text] = putSkill.mock.calls[0]!;
    expect(name).toBe("pdf-forms");
    expect(text).toMatch(/^---\nname: pdf-forms\ndescription: Replace this with what the skill does and when to use it\.\n---\n/);
    await waitFor(() => expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/skills/pdf-forms"));
    expect(await skillsGroup().findByRole("link", { name: "pdf-forms" })).toBeTruthy();
  });

  it("refuses a name a skill already has", async () => {
    skillList = [skill("pdf-forms")];
    await shown();
    await skillsGroup().findByRole("link", { name: "pdf-forms" });
    setCreatingSkill(true);
    const box = await skillsGroup().findByLabelText("New skill name");
    fireEvent.input(box, { target: { value: "pdf forms" } });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await nav().findByRole("alert")).toHaveTextContent("There is already a skill called pdf-forms.");
    expect(putSkill).not.toHaveBeenCalled();
  });

  it("imports a zip, asks before replacing a skill of that name, and opens it", async () => {
    await shown();
    importSkill
      .mockRejectedValueOnce(new ApiError(409, { error: "exists", name: "pdf-forms", message: "there is already a skill pdf-forms" }))
      .mockImplementationOnce(async () => {
        skillList = [skill("pdf-forms")];
        return { ...skill("pdf-forms"), body: "", files: [] };
      });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const zip = new File([new Uint8Array([80, 75, 5, 6])], "pdf-forms.zip", { type: "application/zip" });
    fireEvent.change(skillsGroup().getByLabelText("Skill zip"), { target: { files: [zip] } });
    await waitFor(() => expect(importSkill).toHaveBeenCalledTimes(2));
    expect(importSkill.mock.calls[0]).toEqual([zip, "pdf-forms.zip"]);
    expect(importSkill.mock.calls[1]).toEqual([zip, "pdf-forms.zip", true]);
    expect(confirm).toHaveBeenCalledWith("There is already a skill called pdf-forms. Replace it with the imported one?");
    await waitFor(() => expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/skills/pdf-forms"));
    confirm.mockRestore();
  });

  it("imports a folder as a zip of its files under their relative paths", async () => {
    await shown();
    importSkill.mockImplementation(async () => ({ ...skill("notes-helper"), body: "", files: [] }));
    const md = new File(["---\nname: notes-helper\n---\n"], "SKILL.md");
    Object.defineProperty(md, "webkitRelativePath", { value: "notes-helper/SKILL.md" });
    const script = new File(["echo hi"], "run.sh");
    Object.defineProperty(script, "webkitRelativePath", { value: "notes-helper/scripts/run.sh" });
    fireEvent.change(skillsGroup().getByLabelText("Skill folder"), { target: { files: [md, script] } });
    await waitFor(() => expect(importSkill).toHaveBeenCalled());
    const [blob, filename] = importSkill.mock.calls[0]!;
    expect(filename).toBe("notes-helper.zip");
    const text = new TextDecoder("latin1").decode(new Uint8Array(await (blob as Blob).arrayBuffer()));
    expect(text).toContain("notes-helper/SKILL.md");
    expect(text).toContain("notes-helper/scripts/run.sh");
    await waitFor(() => expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/skills/notes-helper"));
  });

  it("is read only in the apps: the skills are listed with no +", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    skillList = [skill("pdf-forms")];
    await shown();
    expect(await skillsGroup().findByRole("link", { name: "pdf-forms" })).toBeTruthy();
    await waitFor(() => expect(skillsGroup().queryByRole("button", { name: "Add skill" })).toBeNull());
  });
});
