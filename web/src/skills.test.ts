// The skill helpers (skills S4, S5): the name a typed skill name becomes, SKILL.md written back
// from the form with the keys it does not know kept, the folder zip, the file tree and which
// files open as text, and the command palette's New skill and Open skill actions.

import { describe, expect, it } from "vitest";
import type { SkillSummary } from "./api";
import * as M from "./workspace/model";
import { workspace } from "./workspace/store";
import { agentActions } from "./palette/actions";
import {
  crc32,
  creatingSkill,
  setCreatingSkill,
  skillMatches,
  fileTree,
  isTextFile,
  newSkillText,
  skillNameFrom,
  skillPath,
  skillText,
  unknownFrontmatter,
  yamlString,
  zip,
} from "./skills";

describe("skillNameFrom", () => {
  it("makes a name the Agent Skills specification allows", () => {
    expect(skillNameFrom("PDF Forms")).toBe("pdf-forms");
    expect(skillNameFrom("  Café -- Digest! ")).toBe("cafe-digest");
    expect(skillNameFrom("-x-")).toBe("x");
    expect(skillNameFrom("!!!")).toBe("");
    expect(skillNameFrom(`${"a".repeat(63)} b`)).toBe("a".repeat(63));
  });
});

describe("SKILL.md from the form", () => {
  const previous = `---
name: pdf-forms
description: Old
x-owner: wil
tags:
  - pdf
  - forms
license: MIT
---
Body
`;

  it("finds the frontmatter keys the form does not edit, each with its lines", () => {
    expect(unknownFrontmatter(previous)).toEqual(["x-owner: wil", "tags:\n  - pdf\n  - forms"]);
    expect(unknownFrontmatter("no frontmatter")).toEqual([]);
  });

  it("writes the specification's keys, leaves out empty ones, and keeps the unknown keys", () => {
    const text = skillText(
      {
        name: "pdf-forms",
        description: "Fill PDF forms: use when a form arrives.",
        license: "",
        compatibility: "Needs python3",
        metadata: [
          ["author", "Wil"],
          ["", "dropped"],
          ["version", "1.0"],
        ],
        allowed_tools: "Bash(python:*) Read",
        body: "# PDF forms\n\nSteps.\n",
      },
      previous,
    );
    expect(text).toBe(`---
name: pdf-forms
description: "Fill PDF forms: use when a form arrives."
compatibility: Needs python3
metadata:
  author: Wil
  version: "1.0"
allowed-tools: "Bash(python:*) Read"
x-owner: wil
tags:
  - pdf
  - forms
---

# PDF forms

Steps.
`);
  });

  it("quotes a string that would not read back as itself", () => {
    expect(yamlString("plain words")).toBe("plain words");
    expect(yamlString("true")).toBe('"true"');
    expect(yamlString("a: b")).toBe('"a: b"');
    expect(yamlString("- item")).toBe('"- item"');
    expect(yamlString("two\nlines")).toBe('"two\\nlines"');
    expect(yamlString("")).toBe('""');
  });

  it("starts a new skill with the placeholder description", () => {
    expect(newSkillText("pdf-forms")).toMatch(/^---\nname: pdf-forms\ndescription: Replace this with what the skill does and when to use it\.\n---\n\n# pdf-forms\n/);
  });
});

describe("the folder zip", () => {
  it("computes the zip CRC-32", () => {
    expect(crc32(new TextEncoder().encode("123456789"))).toBe(0xcbf43926);
  });

  it("stores each file under its path, with a central directory", async () => {
    const blob = zip([
      { path: "s/SKILL.md", data: new TextEncoder().encode("hi") },
      { path: "s/a.bin", data: new Uint8Array([0, 1, 2]) },
    ]);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const view = new DataView(bytes.buffer);
    expect(view.getUint32(0, true)).toBe(0x04034b50);
    const end = bytes.length - 22;
    expect(view.getUint32(end, true)).toBe(0x06054b50);
    expect(view.getUint16(end + 10, true)).toBe(2);
    const dir = view.getUint32(end + 16, true);
    expect(view.getUint32(dir, true)).toBe(0x02014b50);
    expect(new TextDecoder().decode(bytes.slice(30, 30 + 10))).toBe("s/SKILL.md");
  });
});

describe("the Files tab's helpers", () => {
  const files = [
    { path: "scripts/run.sh", size: 10, media_type: "application/x-sh" },
    { path: "SKILL.md", size: 300, media_type: "text/markdown" },
    { path: "assets/logo.png", size: 2048, media_type: "image/png" },
    { path: "README.txt", size: 5, media_type: "text/plain" },
    { path: "references/deep/notes.md", size: 5, media_type: "text/markdown" },
  ];

  it("lays the files out as a tree: SKILL.md, then folders, then files", () => {
    const rows = fileTree(files, ["empty"]);
    expect(rows.map((r) => `${"  ".repeat(r.depth)}${r.name}${r.folder ? "/" : ""}`)).toEqual([
      "SKILL.md",
      "assets/",
      "  logo.png",
      "empty/",
      "references/",
      "  deep/",
      "    notes.md",
      "scripts/",
      "  run.sh",
      "README.txt",
    ]);
  });

  it("opens text files in the editor and not binary or very large ones", () => {
    expect(isTextFile(files[0]!)).toBe(true);
    expect(isTextFile(files[2]!)).toBe(false);
    expect(isTextFile({ path: "data.yaml", size: 3, media_type: "application/octet-stream" })).toBe(true);
    expect(isTextFile({ path: "huge.txt", size: 3 * 1024 * 1024, media_type: "text/plain" })).toBe(false);
  });

  it("keeps a path inside the skill's folder", () => {
    expect(skillPath("/scripts/run.sh/")).toBe("scripts/run.sh");
    expect(() => skillPath("../etc/passwd")).toThrow(/not a path inside/);
    expect(() => skillPath("a//b")).toThrow();
    expect(() => skillPath("a\\b")).toThrow();
  });
});

describe("the palette's skill actions", () => {
  const skills: SkillSummary[] = [
    { name: "pdf-forms", description: "Fill forms", license: null, compatibility: null, metadata: null, allowed_tools: null, files: 1, size: 1, valid: true, issues: [] },
    { name: "broken", description: null, license: null, compatibility: null, metadata: null, allowed_tools: null, files: 1, size: 1, valid: false, issues: [] },
  ];

  it("offers New skill and Open skill for each", () => {
    const actions = agentActions([], false, skills).filter((a) => a.id.startsWith("skill:"));
    expect(actions.map((a) => a.label)).toEqual(["New skill", "Open skill pdf-forms", "Open skill broken"]);
    expect(actions[1]!.href).toBe("/skills/pdf-forms");
    expect(actions[1]!.icon).toEqual({ type: "book-open" });
    expect(actions[2]!.icon).toEqual({ type: "triangle-alert" });
    expect(typeof actions[0]!.run).toBe("function");
  });

  it("New skill opens the Skills page with its name box shown", () => {
    const create = agentActions([], false, skills).find((a) => a.id === "skill:new")!;
    create.run!();
    expect(creatingSkill()).toBe(true);
    const ws = workspace();
    expect(M.location(ws, M.activePane(ws))).toBe("/skills");
    setCreatingSkill(false);
  });

  it("matches a search by every word in the name, description or license", () => {
    expect(skillMatches(skills[0]!, "")).toBe(true);
    expect(skillMatches(skills[0]!, "PDF fill")).toBe(true);
    expect(skillMatches(skills[0]!, "pdf daily")).toBe(false);
    expect(skillMatches({ ...skills[0]!, license: "MIT" }, "mit")).toBe(true);
  });

  it("leaves out New skill in the apps", () => {
    expect(agentActions([], true, skills).map((a) => a.label)).toEqual(["Open skill pdf-forms", "Open skill broken"]);
  });
});
