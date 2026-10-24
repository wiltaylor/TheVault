// The skill screen (skills S5): the Instructions tab (the frontmatter as a form with the name read
// only, the description's count, metadata rows and allowed-tools; the body with a preview; one
// Save writing SKILL.md with the keys the form does not know kept; issues shown beside their
// field; the placeholder description refused), the Files tab (the tree, a text file edited and
// saved, a binary file's size, type and Download, upload into a folder, a new folder, rename and
// delete after a confirm), the ⋯ menu's Export and Delete skill, and a read-only screen in the apps.

import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Skill } from "../api";

const skill = vi.fn();
const skillsList = vi.fn();
const skillFile = vi.fn();
const putSkill = vi.fn();
const putSkillFile = vi.fn();
const deleteSkillFile = vi.fn();
const deleteSkill = vi.fn();
const appInfo = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: { ...real.api, skill, skills: skillsList, skillFile, putSkill, putSkillFile, deleteSkillFile, deleteSkill },
  };
});
vi.mock("../inApp", () => ({ appInfo }));

const { SkillScreen } = await import("./SkillScreen");
const { ApiError } = await import("../api");
const { DESCRIPTION_PLACEHOLDER } = await import("../skills");

const RAW = `---
name: pdf-forms
description: Fill PDF forms.
x-owner: wil
metadata:
  author: Wil
---

# PDF forms

Run scripts/fill.py.
`;

let stored: Skill;
let contents: Record<string, string>;

function base(): Skill {
  return {
    name: "pdf-forms",
    description: "Fill PDF forms.",
    license: null,
    compatibility: null,
    metadata: { author: "Wil" },
    allowed_tools: "Read",
    valid: true,
    size: 4000,
    issues: [{ at: "x-owner", message: "x-owner is not a key the specification names; it is kept", severity: "warning" }],
    body: "\n# PDF forms\n\nRun scripts/fill.py.\n",
    files: [
      { path: "SKILL.md", size: 120, media_type: "text/markdown" },
      { path: "scripts/fill.py", size: 30, media_type: "text/x-python" },
      { path: "assets/form.pdf", size: 2048, media_type: "application/pdf" },
    ],
  };
}

beforeEach(() => {
  stored = base();
  contents = { "SKILL.md": RAW, "scripts/fill.py": "print('fill')\n" };
  skill.mockImplementation(async () => structuredClone(stored));
  skillsList.mockImplementation(async () => [{ ...stored, files: stored.files.length }]);
  skillFile.mockImplementation(async (_n: string, path: string) => new Blob([contents[path] ?? "\x00\x01"]));
  putSkill.mockResolvedValue(undefined);
  putSkillFile.mockImplementation(async (_n: string, path: string, b: Blob) => {
    contents[path] = await b.text();
    if (!stored.files.some((f) => f.path === path)) stored.files.push({ path, size: b.size, media_type: "text/plain" });
  });
  deleteSkillFile.mockImplementation(async (_n: string, path: string) => {
    stored.files = stored.files.filter((f) => f.path !== path);
  });
  deleteSkill.mockResolvedValue(undefined);
  appInfo.mockResolvedValue(null);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

async function show(path = "/skills/pdf-forms") {
  window.history.replaceState({}, "", path);
  render(() => (
    <Router>
      <Route path={["/skills/:name", "/skills/:name/:tab"]} component={SkillScreen} />
      <Route path="/daily" component={() => <p>Daily</p>} />
    </Router>
  ));
  await screen.findByRole("heading", { name: "pdf-forms" });
}

function tab(name: string) {
  return within(screen.getByRole("region", { name }));
}

describe("the Instructions tab", () => {
  it("shows the frontmatter as a form and the body, the name read only", async () => {
    await show();
    const t = tab("Instructions");
    expect(t.getByLabelText("Name")).toHaveValue("pdf-forms");
    expect(t.getByLabelText("Name")).toHaveAttribute("readonly");
    expect(t.getByLabelText("Description")).toHaveValue("Fill PDF forms.");
    expect(t.getByText(/^15 of 1024 characters/)).toBeTruthy();
    expect(t.getByLabelText("Key 1")).toHaveValue("author");
    expect(t.getByLabelText("Value 1")).toHaveValue("Wil");
    expect(t.getByLabelText("Allowed tools")).toHaveValue("Read");
    expect(t.getByLabelText("Instructions")).toHaveValue("\n# PDF forms\n\nRun scripts/fill.py.\n");
    // A warning without a field of its own shows above the form.
    expect(within(t.getByRole("list", { name: "Issues" })).getByText(/x-owner: x-owner is not a key/)).toBeTruthy();
  });

  it("previews the body as Markdown", async () => {
    await show();
    fireEvent.click(tab("Instructions").getByRole("button", { name: "Preview" }));
    const preview = tab("Instructions").getByLabelText("Preview");
    expect(within(preview).getByRole("heading", { name: "PDF forms" })).toBeTruthy();
    fireEvent.click(tab("Instructions").getByRole("button", { name: "Edit" }));
    expect(tab("Instructions").getByLabelText("Instructions")).toBeTruthy();
  });

  it("saves SKILL.md with one Save, keeping the keys the form does not know", async () => {
    await show();
    const t = tab("Instructions");
    const save = t.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    fireEvent.input(t.getByLabelText("Description"), { target: { value: "Fill PDF forms. Use when a form arrives." } });
    fireEvent.input(t.getByLabelText("License"), { target: { value: "MIT" } });
    fireEvent.click(t.getByRole("button", { name: "Add metadata" }));
    fireEvent.input(t.getByLabelText("Key 2"), { target: { value: "version" } });
    fireEvent.input(t.getByLabelText("Value 2"), { target: { value: "2" } });
    putSkill.mockImplementation(async () => {
      stored = { ...stored, description: "Fill PDF forms. Use when a form arrives.", license: "MIT", metadata: { author: "Wil", version: "2" } };
    });
    fireEvent.click(save);
    await waitFor(() => expect(putSkill).toHaveBeenCalled());
    expect(putSkill.mock.calls[0]).toEqual([
      "pdf-forms",
      `---
name: pdf-forms
description: Fill PDF forms. Use when a form arrives.
license: MIT
metadata:
  author: Wil
  version: "2"
allowed-tools: Read
x-owner: wil
---

# PDF forms

Run scripts/fill.py.
`,
    ]);
    expect(await tab("Instructions").findByRole("status")).toHaveTextContent("Saved.");
  });

  it("shows the issues of a refused save beside their field", async () => {
    await show();
    const t = tab("Instructions");
    fireEvent.input(t.getByLabelText("Description"), { target: { value: "x".repeat(1100) } });
    expect(t.getByText(/^1100 of 1024 characters/)).toHaveClass("error");
    putSkill.mockRejectedValue(
      new ApiError(400, {
        error: "invalid",
        message: "SKILL.md does not check",
        issues: [{ at: "description", message: "description is longer than 1024 characters", severity: "error" }],
      }),
    );
    fireEvent.click(t.getByRole("button", { name: "Save" }));
    expect(await t.findByRole("alert")).toHaveTextContent("description: description is longer than 1024 characters");
    const description = t.getByLabelText("Description").parentElement!;
    expect(within(description).getByText("description is longer than 1024 characters")).toHaveClass("error");
  });

  it("refuses to save until the placeholder description is replaced", async () => {
    stored = { ...base(), description: DESCRIPTION_PLACEHOLDER, issues: [] };
    await show();
    const t = tab("Instructions");
    expect(t.getByText("Replace the placeholder description before saving.")).toBeTruthy();
    fireEvent.input(t.getByLabelText("Instructions"), { target: { value: "# Steps\n" } });
    fireEvent.click(t.getByRole("button", { name: "Save" }));
    expect(await t.findByRole("alert")).toHaveTextContent("Replace the placeholder description");
    expect(putSkill).not.toHaveBeenCalled();
  });
});

describe("the Files tab", () => {
  async function files() {
    await show("/skills/pdf-forms/files");
    return tab("Files");
  }

  it("shows the folder as a tree, SKILL.md first", async () => {
    const t = await files();
    const tree = within(t.getByRole("list", { name: "Skill files" }));
    expect(tree.getAllByRole("listitem").map((li) => li.querySelector(".skill-tree-name")!.textContent!.trim())).toEqual([
      "SKILL.md",
      "assets",
      "form.pdf",
      "scripts",
      "fill.py",
    ]);
    expect(t.queryByRole("button", { name: "Delete SKILL.md" })).toBeNull();
  });

  it("opens a text file in an editor and saves it", async () => {
    const t = await files();
    fireEvent.click(t.getByRole("button", { name: "fill.py" }));
    const editor = await t.findByLabelText("Contents of scripts/fill.py");
    await waitFor(() => expect(editor).toHaveValue("print('fill')\n"));
    fireEvent.input(editor, { target: { value: "print('filled')\n" } });
    fireEvent.click(within(screen.getByRole("region", { name: "scripts/fill.py" })).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putSkillFile).toHaveBeenCalledWith("pdf-forms", "scripts/fill.py", expect.any(Blob)));
    await waitFor(() => expect(contents["scripts/fill.py"]).toBe("print('filled')\n"));
  });

  it("shows a binary file's size and type with Download", async () => {
    const t = await files();
    fireEvent.click(t.getByRole("button", { name: "form.pdf" }));
    const view = within(await screen.findByRole("region", { name: "assets/form.pdf" }));
    expect(view.getByText("2.0 KB, application/pdf")).toBeTruthy();
    expect(view.getByRole("link", { name: "Download" })).toHaveAttribute("href", "/api/skills/pdf-forms/files/assets/form.pdf");
    expect(view.queryByRole("textbox")).toBeNull();
    expect(skillFile).not.toHaveBeenCalledWith("pdf-forms", "assets/form.pdf");
  });

  it("uploads files into the chosen folder", async () => {
    const t = await files();
    fireEvent.change(t.getByLabelText("Folder"), { target: { value: "scripts" } });
    const file = new File(["print(1)"], "extra.py");
    fireEvent.change(t.getByLabelText("Files to upload"), { target: { files: [file] } });
    await waitFor(() => expect(putSkillFile).toHaveBeenCalledWith("pdf-forms", "scripts/extra.py", file));
    expect(await t.findByRole("button", { name: "extra.py" })).toBeTruthy();
  });

  it("makes a folder that files can be uploaded into", async () => {
    const t = await files();
    vi.spyOn(window, "prompt").mockReturnValue("references");
    fireEvent.click(t.getByRole("button", { name: "New folder" }));
    expect(await t.findByRole("button", { name: "references" })).toBeTruthy();
    expect(t.getByLabelText("Folder")).toHaveValue("references");
    fireEvent.change(t.getByLabelText("Files to upload"), { target: { files: [new File(["# Ref"], "guide.md")] } });
    await waitFor(() => expect(putSkillFile).toHaveBeenCalledWith("pdf-forms", "references/guide.md", expect.any(File)));
  });

  it("renames a file by writing it under the new path and deleting the old one", async () => {
    const t = await files();
    vi.spyOn(window, "prompt").mockReturnValue("scripts/fill_form.py");
    fireEvent.click(t.getByRole("button", { name: "Rename scripts/fill.py" }));
    await waitFor(() => expect(deleteSkillFile).toHaveBeenCalledWith("pdf-forms", "scripts/fill.py"));
    expect(putSkillFile).toHaveBeenCalledWith("pdf-forms", "scripts/fill_form.py", expect.any(Blob));
    expect(contents["scripts/fill_form.py"]).toBe("print('fill')\n");
    expect(await t.findByRole("button", { name: "fill_form.py" })).toBeTruthy();
  });

  it("refuses a rename out of the skill's folder", async () => {
    const t = await files();
    vi.spyOn(window, "prompt").mockReturnValue("../escape.py");
    fireEvent.click(t.getByRole("button", { name: "Rename scripts/fill.py" }));
    expect(await t.findByRole("alert")).toHaveTextContent("not a path inside the skill's folder");
    expect(putSkillFile).not.toHaveBeenCalled();
  });

  it("deletes a file, and a folder with its files, only after the confirm", async () => {
    const t = await files();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(t.getByRole("button", { name: "Delete assets/form.pdf" }));
    expect(confirm).toHaveBeenCalledWith("Delete assets/form.pdf?");
    expect(deleteSkillFile).not.toHaveBeenCalled();
    confirm.mockReturnValue(true);
    fireEvent.click(t.getByRole("button", { name: "Delete scripts" }));
    expect(confirm).toHaveBeenLastCalledWith("Delete the folder scripts and the 1 file in it?");
    await waitFor(() => expect(deleteSkillFile).toHaveBeenCalledWith("pdf-forms", "scripts/fill.py"));
    await waitFor(() => expect(t.queryByRole("button", { name: "fill.py" })).toBeNull());
  });
});

describe("the ⋯ menu", () => {
  it("exports the skill as a zip", async () => {
    await show();
    fireEvent.click(screen.getByRole("button", { name: "Actions for pdf-forms" }));
    const menu = within(screen.getByRole("menu", { name: "Actions for pdf-forms" }));
    const exp = menu.getByRole("menuitem", { name: "Export" });
    expect(exp).toHaveAttribute("href", "/api/skills/pdf-forms/export");
    expect(exp).toHaveAttribute("download", "pdf-forms.zip");
  });

  it("deletes the skill after the confirm", async () => {
    await show();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Actions for pdf-forms" }));
    fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Delete skill" }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Delete the skill pdf-forms?"));
    await waitFor(() => expect(deleteSkill).toHaveBeenCalledWith("pdf-forms"));
    expect(await screen.findByText("Daily")).toBeTruthy();
  });

  it("says when there is no such skill", async () => {
    skill.mockRejectedValue(new ApiError(404, { error: "not_found" }));
    window.history.replaceState({}, "", "/skills/gone");
    render(() => (
      <Router>
        <Route path="/skills/:name" component={SkillScreen} />
      </Router>
    ));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is no skill called gone.");
  });
});

describe("in the apps", () => {
  it("shows the skill read only, with Export but no Save, file changes or Delete skill", async () => {
    appInfo.mockResolvedValue({ app: true, paired: true, device_id: "d", device_name: "Phone", server_url: "https://vault" });
    await show();
    expect(await screen.findByText("Edit skills in the web app. This app can show them but not change them.")).toBeTruthy();
    const t = tab("Instructions");
    expect(t.getByLabelText("Description")).toBeDisabled();
    expect(t.getByLabelText("Instructions")).toHaveAttribute("readonly");
    expect(t.queryByRole("button", { name: "Save" })).toBeNull();
    expect(t.queryByRole("button", { name: "Add metadata" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Actions for pdf-forms" }));
    const menu = within(screen.getByRole("menu"));
    expect(menu.getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Export"]);

    fireEvent.click(screen.getByRole("tab", { name: "Files" }));
    const files = within(await screen.findByRole("region", { name: "Files" }));
    expect(files.queryByRole("button", { name: /Upload files/ })).toBeNull();
    expect(files.queryByRole("button", { name: "Delete assets/form.pdf" })).toBeNull();
    fireEvent.click(files.getByRole("button", { name: "fill.py" }));
    const editor = await files.findByLabelText("Contents of scripts/fill.py");
    expect(editor).toHaveAttribute("readonly");
  });
});
