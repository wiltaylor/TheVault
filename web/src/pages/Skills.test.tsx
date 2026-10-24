// The Skills page (skills S4): every skill as a card with its name linking to the skill screen,
// its description, file count, size and license, an invalid skill's mark and first issue; the
// search box; New skill from a name, opening the new skill; Import from a zip, asking before it
// replaces a skill of that name, and from a folder; the empty state with its link to the docs;
// and a page that only lists in the apps.

import { Route, Router, useParams } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SkillSummary } from "../api";

const skillsList = vi.fn();
const putSkill = vi.fn();
const importSkill = vi.fn();
const appInfo = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, skills: skillsList, putSkill, importSkill } };
});
vi.mock("../inApp", () => ({ appInfo }));

const { Skills } = await import("./Skills");
const { ApiError } = await import("../api");
const { DESCRIPTION_PLACEHOLDER, SKILLS_DOC, setCreatingSkill } = await import("../skills");

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

let list: SkillSummary[];

beforeEach(() => {
  list = [];
  skillsList.mockImplementation(async () => list);
  appInfo.mockResolvedValue(null);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  setCreatingSkill(false);
});

function SkillStub() {
  const params = useParams();
  return <h1>Skill screen {params.name}</h1>;
}

async function show() {
  window.history.replaceState({}, "", "/skills");
  render(() => (
    <Router>
      <Route path="/skills" component={Skills} />
      <Route path="/skills/:name" component={SkillStub} />
    </Router>
  ));
  await screen.findByRole("heading", { name: "Skills" });
  await waitFor(() => expect(skillsList).toHaveBeenCalled());
}

function cards() {
  return within(screen.getByRole("list", { name: "Skills" })).getAllByRole("listitem");
}

const inApp = { app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" };

describe("the Skills page", () => {
  it("lists every skill by name with its description, files, size and license", async () => {
    list = [skill("pdf-forms", { files: 3, size: 2048, license: "MIT" }), skill("notes-helper")];
    await show();
    await waitFor(() => expect(cards()).toHaveLength(2));
    // Sorted by name.
    const links = cards().map((c) => within(c).getByRole("link"));
    expect(links.map((l) => l.textContent)).toEqual(["notes-helper", "pdf-forms"]);
    expect(links[1]!).toHaveAttribute("href", "/skills/pdf-forms");
    const pdf = within(cards()[1]!);
    expect(pdf.getByText("What pdf-forms does.")).toHaveClass("skill-card-description");
    expect(cards()[1]!).toHaveTextContent("3 files · 2.0 KB · MIT");
    expect(cards()[0]!).toHaveTextContent("1 file · 120 B");
    expect(cards()[0]!).not.toHaveTextContent("·  ·");
  });

  it("marks an invalid skill with its first issue", async () => {
    list = [
      skill("broken", {
        valid: false,
        issues: [
          { at: "x-owner", message: "unknown key", severity: "warning" },
          { at: "name", message: "name must match the folder", severity: "error" },
        ],
      }),
      skill("pdf-forms"),
    ];
    await show();
    await waitFor(() => expect(cards()).toHaveLength(2));
    const broken = within(cards()[0]!);
    expect(broken.getByText("Invalid")).toBeTruthy();
    expect(broken.getByText("name: name must match the folder")).toHaveClass("error");
    expect(within(cards()[1]!).queryByText("Invalid")).toBeNull();
  });

  it("narrows the list with the search box, by name, description or license", async () => {
    list = [skill("pdf-forms", { license: "MIT" }), skill("notes-helper", { description: "Tidies daily notes." })];
    await show();
    await waitFor(() => expect(cards()).toHaveLength(2));
    const search = screen.getByRole("searchbox", { name: "Search skills" });
    fireEvent.input(search, { target: { value: "daily NOTES" } });
    expect(cards().map((c) => within(c).getByRole("link").textContent)).toEqual(["notes-helper"]);
    fireEvent.input(search, { target: { value: "mit" } });
    expect(cards().map((c) => within(c).getByRole("link").textContent)).toEqual(["pdf-forms"]);
    fireEvent.input(search, { target: { value: "nothing like it" } });
    expect(screen.getByText("No skills match.")).toBeTruthy();
    fireEvent.keyDown(search, { key: "Escape" });
    expect(cards()).toHaveLength(2);
  });

  it("makes a new skill from a name, with the placeholder description, and opens it", async () => {
    await show();
    putSkill.mockImplementation(async (name: string) => {
      list = [skill(name, { description: DESCRIPTION_PLACEHOLDER })];
    });
    fireEvent.click(await screen.findByRole("button", { name: "New skill" }));
    const box = screen.getByLabelText("New skill name");
    fireEvent.input(box, { target: { value: "PDF Forms" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(putSkill).toHaveBeenCalled());
    const [name, text] = putSkill.mock.calls[0]!;
    expect(name).toBe("pdf-forms");
    expect(text).toMatch(/^---\nname: pdf-forms\ndescription: Replace this with what the skill does and when to use it\.\n---\n/);
    expect(await screen.findByRole("heading", { name: "Skill screen pdf-forms" })).toBeTruthy();
  });

  it("shows the name box when opened with it asked for, as the palette's New skill does", async () => {
    setCreatingSkill(true);
    await show();
    expect(await screen.findByLabelText("New skill name")).toBeTruthy();
  });

  it("refuses a name a skill already has", async () => {
    list = [skill("pdf-forms")];
    await show();
    await waitFor(() => expect(cards()).toHaveLength(1));
    fireEvent.click(await screen.findByRole("button", { name: "New skill" }));
    const box = screen.getByLabelText("New skill name");
    fireEvent.input(box, { target: { value: "pdf forms" } });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByRole("alert")).toHaveTextContent("There is already a skill called pdf-forms.");
    expect(putSkill).not.toHaveBeenCalled();
  });

  it("imports a zip, asks before replacing a skill of that name, and opens it", async () => {
    await show();
    importSkill
      .mockRejectedValueOnce(new ApiError(409, { error: "skill_exists", name: "pdf-forms", message: "there is already a skill pdf-forms" }))
      .mockImplementationOnce(async () => {
        list = [skill("pdf-forms")];
        return { ...skill("pdf-forms"), body: "", files: [] };
      });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(await screen.findByRole("button", { name: "Import" }));
    const menu = within(screen.getByRole("menu", { name: "Import skill" }));
    expect(menu.getAllByRole("menuitem").map((b) => b.textContent)).toContain("From a zip");
    const zip = new File([new Uint8Array([80, 75, 5, 6])], "pdf-forms.zip", { type: "application/zip" });
    fireEvent.change(screen.getByLabelText("Skill zip"), { target: { files: [zip] } });
    await waitFor(() => expect(importSkill).toHaveBeenCalledTimes(2));
    expect(importSkill.mock.calls[0]).toEqual([zip, "pdf-forms.zip"]);
    expect(importSkill.mock.calls[1]).toEqual([zip, "pdf-forms.zip", true]);
    expect(confirm).toHaveBeenCalledWith("There is already a skill called pdf-forms. Replace it with the imported one?");
    expect(await screen.findByRole("heading", { name: "Skill screen pdf-forms" })).toBeTruthy();
  });

  it("writes nothing when the replace is declined", async () => {
    await show();
    importSkill.mockRejectedValueOnce(new ApiError(409, { error: "skill_exists", name: "pdf-forms" }));
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const zip = new File([new Uint8Array([80, 75, 5, 6])], "pdf-forms.zip", { type: "application/zip" });
    fireEvent.change(await screen.findByLabelText("Skill zip"), { target: { files: [zip] } });
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    expect(importSkill).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("heading", { name: "Skills" })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("imports a folder as a zip of its files under their relative paths", async () => {
    await show();
    importSkill.mockImplementation(async () => ({ ...skill("notes-helper"), body: "", files: [] }));
    const md = new File(["---\nname: notes-helper\n---\n"], "SKILL.md");
    Object.defineProperty(md, "webkitRelativePath", { value: "notes-helper/SKILL.md" });
    const script = new File(["echo hi"], "run.sh");
    Object.defineProperty(script, "webkitRelativePath", { value: "notes-helper/scripts/run.sh" });
    fireEvent.change(await screen.findByLabelText("Skill folder"), { target: { files: [md, script] } });
    await waitFor(() => expect(importSkill).toHaveBeenCalled());
    const [blob, filename] = importSkill.mock.calls[0]!;
    expect(filename).toBe("notes-helper.zip");
    const text = new TextDecoder("latin1").decode(new Uint8Array(await (blob as Blob).arrayBuffer()));
    expect(text).toContain("notes-helper/SKILL.md");
    expect(text).toContain("notes-helper/scripts/run.sh");
    expect(await screen.findByRole("heading", { name: "Skill screen notes-helper" })).toBeTruthy();
  });

  it("explains skills and links to the docs when there are none", async () => {
    await show();
    expect(await screen.findByText(/No skills yet\. A skill is a folder of instructions/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "How skills work" })).toHaveAttribute("href", SKILLS_DOC);
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("only lists in the apps: no New skill and no Import", async () => {
    appInfo.mockResolvedValue(inApp);
    list = [skill("pdf-forms")];
    await show();
    await waitFor(() => expect(cards()).toHaveLength(1));
    expect(await screen.findByRole("note")).toHaveTextContent("Add and edit skills in the web app.");
    expect(screen.queryByRole("button", { name: "New skill" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import" })).toBeNull();
    expect(screen.queryByLabelText("Skill zip")).toBeNull();
  });
});
