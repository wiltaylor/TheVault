// Skills (skills S4, S5): the list the Skills page, the sidebar's Skills item and the command
// palette show, creating and importing one, and what the skill screen needs to write `SKILL.md` back from
// its form: the frontmatter as YAML, keeping the keys the form does not know as they were.

import { createRoot, createSignal } from "solid-js";
import { ApiError, type Skill, type SkillFile, type SkillSummary, api } from "./api";

/** The icon a skill shows. */
export const SKILL_ICON = "book-open";

/** The Skills page, which lists every skill. */
export const SKILLS_HREF = "/skills";

/** Where docs/skills.md is read, as the page's empty state links to it. */
export const SKILLS_DOC = "https://github.com/wiltaylor/TheVault/blob/main/docs/skills.md";

/** The frontmatter's limits, from the Agent Skills specification (skills S2). */
export const SKILL_NAME_MAX = 64;
export const SKILL_DESCRIPTION_MAX = 1024;
export const SKILL_COMPATIBILITY_MAX = 500;

/** The largest text file the Files tab opens in its editor, as a model may read one (skills S1). */
export const SKILL_TEXT_MAX = 2 * 1024 * 1024;

/** What a new skill's description says until the user replaces it (skills S4). */
export const DESCRIPTION_PLACEHOLDER = "Replace this with what the skill does and when to use it.";

const store = createRoot(() => {
  const [list, setList] = createSignal<SkillSummary[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [version, setVersion] = createSignal(0);
  return { list, setList, error, setError, creating, setCreating, version, setVersion };
});

/** Every skill, by name, as the server last listed them. */
export const skills = store.list;

/** Why the list could not be read, if it could not. */
export const skillsError = store.error;

/** Whether the Skills page shows the new skill's name box. */
export const creatingSkill = store.creating;
export const setCreatingSkill = store.setCreating;

/** Bumped by each change made from the Skills page or a skill screen, so an open skill screen reads its skill again. */
export const skillsVersion = store.version;

/** Read the skill list again. A server without skills (404) lists none. */
export async function refreshSkills(): Promise<void> {
  try {
    const l = await api.skills();
    store.setList([...l].sort((a, b) => a.name.localeCompare(b.name)));
    store.setError(null);
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      store.setList([]);
      store.setError(null);
      return;
    }
    store.setError(e instanceof Error ? e.message : String(e));
  }
}

/** After a change: read the list again and tell open skill screens. */
export async function skillsChanged(): Promise<void> {
  store.setVersion(store.version() + 1);
  await refreshSkills();
}

/**
 * Whether a skill matches what was typed in the Skills page's search box: every word, ignoring
 * case, in its name, description or license.
 */
export function skillMatches(s: SkillSummary, query: string): boolean {
  const hay = [s.name, s.description ?? "", s.license ?? ""].join("\n").toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

/** The address of a skill's screen. */
export function skillHref(name: string, tab?: string): string {
  return `/skills/${encodeURIComponent(name)}${tab ? `/${tab}` : ""}`;
}

/**
 * The skill name for what was typed: lower case, runs of anything but a-z and 0-9 as one `-`, no
 * hyphen at either end, at most 64 characters, as the specification allows. Empty when nothing is
 * left.
 */
export function skillNameFrom(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, SKILL_NAME_MAX)
    .replace(/-+$/, "");
}

/** A new skill's `SKILL.md`: its name, the placeholder description, and a heading. */
export function newSkillText(name: string): string {
  return `---\nname: ${name}\ndescription: ${DESCRIPTION_PLACEHOLDER}\n---\n\n# ${name}\n\nWrite the steps an agent follows when it uses this skill.\n`;
}

/** Create the skill for what was typed, from `skillNameFrom`. Refuses a name a skill has. Answers the name. */
export async function createSkill(text: string): Promise<string> {
  const name = skillNameFrom(text);
  if (!name) throw new Error("Type a name with at least one letter or digit.");
  if (skills().some((s) => s.name === name)) throw new Error(`There is already a skill called ${name}.`);
  await api.putSkill(name, newSkillText(name));
  await skillsChanged();
  return name;
}

/**
 * Import a zipped skill. An existing name asks to replace it first; no answers null. Answers the
 * imported skill's name.
 */
export async function importSkillZip(zip: Blob, filename: string, ask: (q: string) => boolean = confirm): Promise<string | null> {
  let skill: Skill;
  try {
    skill = await api.importSkill(zip, filename);
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 409)) throw e;
    const name = typeof e.body.name === "string" ? e.body.name : null;
    if (!ask(name ? `There is already a skill called ${name}. Replace it with the imported one?` : `${e.message} Replace it with the imported one?`)) {
      return null;
    }
    skill = await api.importSkill(zip, filename, true);
  }
  await skillsChanged();
  return skill.name;
}

/** Import a folder chosen with a directory picker: its files zipped under their relative paths. */
export async function importSkillFolder(files: readonly File[], ask?: (q: string) => boolean): Promise<string | null> {
  if (files.length === 0) return null;
  const entries = await Promise.all(
    files.map(async (f) => ({ path: (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name, data: new Uint8Array(await f.arrayBuffer()) })),
  );
  const top = entries[0]!.path.split("/")[0] ?? "skill";
  return importSkillZip(zip(entries), `${top}.zip`, ask);
}

/** Whether this browser can choose a folder. */
export function canPickFolder(): boolean {
  return typeof HTMLInputElement !== "undefined" && "webkitdirectory" in HTMLInputElement.prototype;
}

// ---- zip ------------------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(data: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of data) c = CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of `entries`, stored without compression, each path in UTF-8. */
export function zip(entries: readonly { path: string; data: Uint8Array }[]): Blob {
  const utf8 = new TextEncoder();
  const parts: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const e of entries) {
    const name = utf8.encode(e.path);
    const crc = crc32(e.data);
    const local = new Uint8Array(30 + name.length);
    const lv = new DataView(local.buffer);
    lv.setUint32(0, 0x04034b50, true);
    lv.setUint16(4, 20, true);
    lv.setUint16(6, 0x0800, true); // UTF-8 names
    lv.setUint16(8, 0, true); // stored
    lv.setUint32(14, crc, true);
    lv.setUint32(18, e.data.length, true);
    lv.setUint32(22, e.data.length, true);
    lv.setUint16(26, name.length, true);
    local.set(name, 30);
    const dir = new Uint8Array(46 + name.length);
    const dv = new DataView(dir.buffer);
    dv.setUint32(0, 0x02014b50, true);
    dv.setUint16(4, 20, true);
    dv.setUint16(6, 20, true);
    dv.setUint16(8, 0x0800, true);
    dv.setUint32(16, crc, true);
    dv.setUint32(20, e.data.length, true);
    dv.setUint32(24, e.data.length, true);
    dv.setUint16(28, name.length, true);
    dv.setUint32(42, offset, true);
    dir.set(name, 46);
    parts.push(local, e.data);
    central.push(dir);
    offset += local.length + e.data.length;
  }
  const size = central.reduce((n, c) => n + c.length, 0);
  const end = new Uint8Array(22);
  const ev = new DataView(end.buffer);
  ev.setUint32(0, 0x06054b50, true);
  ev.setUint16(8, entries.length, true);
  ev.setUint16(10, entries.length, true);
  ev.setUint32(12, size, true);
  ev.setUint32(16, offset, true);
  return new Blob([...parts, ...central, end] as BlobPart[], { type: "application/zip" });
}

// ---- SKILL.md -------------------------------------------------------------------------------

/** What the Instructions tab edits: the frontmatter the specification names, and the body. */
export interface SkillForm {
  name: string;
  description: string;
  license: string;
  compatibility: string;
  /** Key-value rows, in order. A row with an empty key is left out of the file. */
  metadata: [string, string][];
  allowed_tools: string;
  body: string;
}

export function skillForm(s: Skill): SkillForm {
  return {
    name: s.name,
    description: s.description ?? "",
    license: s.license ?? "",
    compatibility: s.compatibility ?? "",
    metadata: Object.entries(s.metadata ?? {}),
    allowed_tools: s.allowed_tools ?? "",
    body: s.body,
  };
}

const KNOWN_KEYS = new Set(["name", "description", "license", "compatibility", "metadata", "allowed-tools"]);

/**
 * The top-level entries of `SKILL.md`'s frontmatter that the form does not edit, each as its
 * lines of YAML, in order. Empty when there is no frontmatter.
 */
export function unknownFrontmatter(text: string): string[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  if (lines[0]?.trim() !== "---") return [];
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) return [];
  const out: string[] = [];
  let key: string | null = null;
  let entry: string[] = [];
  const flush = () => {
    if (key !== null && !KNOWN_KEYS.has(key)) out.push(entry.join("\n").replace(/\n+$/, ""));
    key = null;
    entry = [];
  };
  for (const line of lines.slice(1, end)) {
    const top = /^([^\s#:][^:]*?|"[^"]*"|'[^']*'):(\s|$)/.exec(line);
    if (top) {
      flush();
      key = top[1]!.replace(/^["']|["']$/g, "");
      entry = [line];
    } else if (key !== null) {
      entry.push(line);
    }
  }
  flush();
  return out;
}

/** `s` as a YAML scalar: plain when that reads back as the same string, else double-quoted. */
export function yamlString(s: string): string {
  const plain =
    s !== "" &&
    s === s.trim() &&
    !/[\n\r\t"'\\#:{}[\],&*!|>%@`]/.test(s) &&
    !/^[-?]/.test(s) &&
    !/^(true|false|yes|no|on|off|null|~|[-+]?(\d[\d_]*(\.\d*)?|\.\d+)([eE][-+]?\d+)?)$/i.test(s);
  return plain ? s : JSON.stringify(s);
}

/**
 * The `SKILL.md` for `form`: the frontmatter, the specification's keys first and empty ones left
 * out, then the keys of `previous` the form does not know as they were, then the body.
 */
export function skillText(form: SkillForm, previous = ""): string {
  const fm = [`name: ${yamlString(form.name)}`, `description: ${yamlString(form.description.trim())}`];
  if (form.license.trim()) fm.push(`license: ${yamlString(form.license.trim())}`);
  if (form.compatibility.trim()) fm.push(`compatibility: ${yamlString(form.compatibility.trim())}`);
  const meta = form.metadata.filter(([k]) => k.trim());
  if (meta.length) {
    fm.push("metadata:");
    for (const [k, v] of meta) fm.push(`  ${yamlString(k.trim())}: ${yamlString(v)}`);
  }
  if (form.allowed_tools.trim()) fm.push(`allowed-tools: ${yamlString(form.allowed_tools.trim())}`);
  fm.push(...unknownFrontmatter(previous));
  const body = form.body.startsWith("\n") || form.body === "" ? form.body : `\n${form.body}`;
  return `---\n${fm.join("\n")}\n---\n${body}`;
}

// ---- files ----------------------------------------------------------------------------------

const TEXT_TYPES = /^(text\/|application\/(json|(x-)?yaml|toml|xml|javascript|x-sh|x-shellscript|x-python|ecmascript|sql|x-httpd-php)\b)|\+(json|xml)\b/;
const TEXT_EXTENSIONS = new Set(
  "md markdown txt text py sh bash zsh js mjs cjs ts tsx jsx json jsonl yaml yml toml ini cfg conf csv tsv html htm css xml svg rb go rs java kt c h cpp hpp lua pl r sql env gitignore license makefile dockerfile readme".split(" "),
);

/** Whether the Files tab opens `f` in its text editor: a text type or extension, and not too large. */
export function isTextFile(f: Pick<SkillFile, "path" | "media_type" | "size">): boolean {
  if (f.size > SKILL_TEXT_MAX) return false;
  if (TEXT_TYPES.test(f.media_type)) return true;
  const base = f.path.split("/").pop() ?? "";
  const ext = base.includes(".") ? base.split(".").pop()!.toLowerCase() : base.toLowerCase();
  return TEXT_EXTENSIONS.has(ext);
}

/**
 * `path` as a path under a skill's folder: `/`-separated, no empty, `.` or `..` part, no
 * backslash. Throws when it is not one.
 */
export function skillPath(path: string): string {
  const parts = path.trim().replace(/^\/+|\/+$/g, "").split("/");
  if (parts.some((p) => p === "" || p === "." || p === ".." || p.includes("\\"))) {
    throw new Error(`${path.trim() || "An empty name"} is not a path inside the skill's folder.`);
  }
  return parts.join("/");
}

/** One row of the Files tab's tree: a folder or a file, with its depth. */
export interface TreeRow {
  path: string;
  name: string;
  depth: number;
  folder: boolean;
  file?: SkillFile;
}

/**
 * The skill's files as a tree, depth first: `SKILL.md` first, then folders before files, each by
 * name. `folders` adds folders that hold no file yet.
 */
export function fileTree(files: readonly SkillFile[], folders: readonly string[] = []): TreeRow[] {
  const dirs = new Set<string>();
  const addDirs = (p: string) => {
    const parts = p.split("/");
    for (let i = 1; i <= parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
  };
  for (const f of files) if (f.path.includes("/")) addDirs(f.path.slice(0, f.path.lastIndexOf("/")));
  for (const d of folders) addDirs(d);
  const rows: TreeRow[] = [];
  const walk = (dir: string, depth: number) => {
    const prefix = dir ? `${dir}/` : "";
    const sub = [...dirs].filter((d) => d.startsWith(prefix) && !d.slice(prefix.length).includes("/") && d !== dir).sort();
    const here = files
      .filter((f) => f.path.startsWith(prefix) && !f.path.slice(prefix.length).includes("/"))
      .sort((a, b) => (a.path === "SKILL.md" ? -1 : b.path === "SKILL.md" ? 1 : a.path.localeCompare(b.path)));
    if (!dir) for (const f of here.filter((f) => f.path === "SKILL.md")) rows.push({ path: f.path, name: f.path, depth, folder: false, file: f });
    for (const d of sub) {
      rows.push({ path: d, name: d.slice(prefix.length), depth, folder: true });
      walk(d, depth + 1);
    }
    for (const f of here) if (dir || f.path !== "SKILL.md") rows.push({ path: f.path, name: f.path.slice(prefix.length), depth, folder: false, file: f });
  };
  walk("", 0);
  return rows;
}

/** A size in bytes for people: bytes, KB or MB. */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
