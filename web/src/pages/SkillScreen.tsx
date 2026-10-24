// The skill screen, `/skills/<name>` (skills S5): one skill as tabs. Instructions, the default,
// edits `SKILL.md` as a form (the frontmatter the Agent Skills specification names, with the name
// read only) and its Markdown body with a preview, one Save writing the whole file, and shows the
// server's issues beside the field they name. Files holds the skill's folder as a tree: a text file
// opens in an editor with Save, a binary one shows its size and type with Download, and files can
// be uploaded into a folder, folders made, and either renamed or deleted. The ⋯ menu exports the
// skill as a zip and deletes it. In the apps the screen is read only: skill files need `admin`,
// which a device token never has (skills S3).

import { useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup } from "solid-js";
import { ApiError, type Skill, type SkillFile, type SkillIssue, api, skillExportUrl, skillFileUrl } from "../api";
import { Markdown } from "../ai/markdown";
import { MenuBox } from "../components/AgentsNav";
import { TypeIcon } from "../components/Icon";
import { appInfo } from "../inApp";
import {
  DESCRIPTION_PLACEHOLDER,
  SKILL_COMPATIBILITY_MAX,
  SKILL_DESCRIPTION_MAX,
  SKILL_ICON,
  type SkillForm,
  fileTree,
  isTextFile,
  sizeText,
  skillForm,
  skillHref,
  skillPath,
  skillText,
  skillsChanged,
  skillsVersion,
} from "../skills";
import { usePane } from "../workspace/pane";
import { SaveBar } from "./AgentScreen";
import { errorText } from "./TypeEditor";

const TABS = [
  { id: "instructions", label: "Instructions" },
  { id: "files", label: "Files" },
] as const;

/** What the tabs are given: the skill as last read, and a way to read it again. */
interface TabProps {
  skill: Skill;
  /** `SKILL.md` as stored, for the frontmatter keys the form does not edit. */
  raw: string;
  reload: () => Promise<void>;
  readOnly: boolean;
  id: string;
}

/** The issues whose `at` is `key`, as a list beside its field. */
function FieldIssues(props: { issues: SkillIssue[]; at: string }) {
  const mine = () => props.issues.filter((i) => i.at === props.at);
  return (
    <Show when={mine().length > 0}>
      <ul class="skill-issues skill-field-issues">
        <For each={mine()}>{(i) => <li class={i.severity === "error" ? "error" : "warning"}>{i.message}</li>}</For>
      </ul>
    </Show>
  );
}

const FIELD_KEYS = ["name", "description", "license", "compatibility", "metadata", "allowed-tools", "allowed_tools", "body"];

function InstructionsTab(p: TabProps) {
  const id = (what: string) => `${p.id}-${what}`;
  const [draft, setDraft] = createSignal<SkillForm>(skillForm(p.skill));
  const [preview, setPreview] = createSignal(false);
  // The issues of the last refused save; until then, the skill's own.
  const [refused, setRefused] = createSignal<SkillIssue[] | null>(null);
  createEffect(
    on(
      () => p.skill,
      (s) => {
        setDraft(skillForm(s));
        setRefused(null);
      },
      { defer: true },
    ),
  );
  const issues = () => refused() ?? p.skill.issues;
  const dirty = createMemo(() => JSON.stringify(draft()) !== JSON.stringify(skillForm(p.skill)));
  const set = (patch: Partial<SkillForm>) => setDraft((d) => ({ ...d, ...patch }));
  const length = (s: string) => [...s].length;
  const placeholder = () => draft().description.trim() === DESCRIPTION_PLACEHOLDER;
  const general = () => issues().filter((i) => !i.at || !FIELD_KEYS.includes(i.at));

  async function save() {
    if (placeholder()) throw new Error("Replace the placeholder description with what the skill does and when to use it.");
    try {
      await api.putSkill(p.skill.name, skillText(draft(), p.raw));
    } catch (e) {
      if (e instanceof ApiError && Array.isArray(e.body.issues)) setRefused(e.body.issues as SkillIssue[]);
      throw e;
    }
    setRefused(null);
    await p.reload();
    void skillsChanged();
  }

  function setMeta(i: number, at: 0 | 1, value: string) {
    set({ metadata: draft().metadata.map((row, j) => (j === i ? ((at === 0 ? [value, row[1]] : [row[0], value]) as [string, string]) : row)) });
  }

  return (
    <section class="agent-tab skill-tab" aria-label="Instructions">
      <Show when={general().length > 0}>
        <ul class="skill-issues" aria-label="Issues">
          <For each={general()}>
            {(i) => (
              <li class={i.severity === "error" ? "error" : "warning"}>
                {i.at ? `${i.at}: ` : ""}
                {i.message}
              </li>
            )}
          </For>
        </ul>
      </Show>
      <fieldset class="agent-fieldset" disabled={p.readOnly}>
        <div class="form-grid">
          <label for={id("name")}>Name</label>
          <div>
            <input id={id("name")} value={draft().name} readOnly aria-describedby={id("name-note")} />
            <p class="muted skill-note" id={id("name-note")}>
              The skill's folder name. Import the skill under another name to change it.
            </p>
            <FieldIssues issues={issues()} at="name" />
          </div>
          <label for={id("description")}>Description</label>
          <div>
            <textarea
              id={id("description")}
              rows={3}
              value={draft().description}
              onInput={(e) => set({ description: e.currentTarget.value })}
            />
            <p class={length(draft().description) > SKILL_DESCRIPTION_MAX || !draft().description.trim() ? "error skill-note" : "muted skill-note"}>
              {length(draft().description)} of {SKILL_DESCRIPTION_MAX} characters. What the skill does and when to use it:
              agents choose skills by this.
            </p>
            <Show when={placeholder()}>
              <p class="warning skill-note" role="note">
                Replace the placeholder description before saving.
              </p>
            </Show>
            <FieldIssues issues={issues()} at="description" />
          </div>
          <label for={id("license")}>License</label>
          <div>
            <input id={id("license")} value={draft().license} onInput={(e) => set({ license: e.currentTarget.value })} />
            <FieldIssues issues={issues()} at="license" />
          </div>
          <label for={id("compatibility")}>Compatibility</label>
          <div>
            <input
              id={id("compatibility")}
              value={draft().compatibility}
              onInput={(e) => set({ compatibility: e.currentTarget.value })}
            />
            <Show when={draft().compatibility}>
              <p class={length(draft().compatibility) > SKILL_COMPATIBILITY_MAX ? "error skill-note" : "muted skill-note"}>
                {length(draft().compatibility)} of {SKILL_COMPATIBILITY_MAX} characters
              </p>
            </Show>
            <FieldIssues issues={issues()} at="compatibility" />
          </div>
          <span>Metadata</span>
          <div class="skill-metadata" role="group" aria-label="Metadata">
            <For each={draft().metadata}>
              {(row, i) => (
                <div class="skill-metadata-row">
                  <input aria-label={`Key ${i() + 1}`} placeholder="Key" value={row[0]} onInput={(e) => setMeta(i(), 0, e.currentTarget.value)} />
                  <input aria-label={`Value ${i() + 1}`} placeholder="Value" value={row[1]} onInput={(e) => setMeta(i(), 1, e.currentTarget.value)} />
                  <Show when={!p.readOnly}>
                    <button
                      type="button"
                      aria-label={`Remove ${row[0] || `row ${i() + 1}`}`}
                      onClick={() => set({ metadata: draft().metadata.filter((_, j) => j !== i()) })}
                    >
                      <TypeIcon name="x" size={14} />
                    </button>
                  </Show>
                </div>
              )}
            </For>
            <Show when={!p.readOnly}>
              <button type="button" class="skill-add-row" onClick={() => set({ metadata: [...draft().metadata, ["", ""]] })}>
                Add metadata
              </button>
            </Show>
            <FieldIssues issues={issues()} at="metadata" />
          </div>
          <label for={id("tools")}>Allowed tools</label>
          <div>
            <input
              id={id("tools")}
              value={draft().allowed_tools}
              placeholder="Space-separated, such as Bash(git:*) Read"
              onInput={(e) => set({ allowed_tools: e.currentTarget.value })}
            />
            <p class="muted skill-note">Recorded as the skill asks. It grants nothing: each agent's own permissions decide its tools.</p>
            <FieldIssues issues={issues()} at="allowed-tools" />
            <FieldIssues issues={issues()} at="allowed_tools" />
          </div>
        </div>
      </fieldset>
      <div class="skill-body-head">
        <label for={id("body")}>Instructions</label>
        <button type="button" aria-pressed={preview()} onClick={() => setPreview(!preview())}>
          <TypeIcon name={preview() ? "pencil" : "eye"} size={14} /> {preview() ? "Edit" : "Preview"}
        </button>
      </div>
      <Show
        when={!preview()}
        fallback={
          <div class="skill-preview" aria-label="Preview">
            <Markdown text={draft().body} />
          </div>
        }
      >
        <textarea
          id={id("body")}
          class="ai-system-prompt skill-body"
          rows={22}
          spellcheck={false}
          readOnly={p.readOnly}
          value={draft().body}
          onInput={(e) => set({ body: e.currentTarget.value })}
        />
      </Show>
      <FieldIssues issues={issues()} at="body" />
      <SaveBar dirty={dirty()} readOnly={p.readOnly} save={save} />
    </section>
  );
}

function FilesTab(p: TabProps & { open: string | null; setOpen: (path: string | null) => void }) {
  // Folders made here that hold no file yet: the server keeps a folder only once a file is in it.
  const [folders, setFolders] = createSignal<string[]>([]);
  const [target, setTarget] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let upload!: HTMLInputElement;
  const rows = createMemo(() => fileTree(p.skill.files, folders()));
  const allFolders = createMemo(() => rows().filter((r) => r.folder).map((r) => r.path));
  const opened = createMemo(() => p.skill.files.find((f) => f.path === p.open) ?? null);
  const name = () => p.skill.name;

  async function run(work: () => Promise<unknown>) {
    setError(null);
    setBusy(true);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      await p.reload().catch(() => {});
      void skillsChanged();
    }
  }

  const exists = (path: string) => p.skill.files.some((f) => f.path === path);
  const under = (dir: string) => p.skill.files.filter((f) => f.path.startsWith(`${dir}/`));
  const join = (dir: string, base: string) => skillPath(dir ? `${dir}/${base}` : base);

  function uploaded(files: File[]) {
    const dir = target();
    void run(async () => {
      for (const f of files) {
        const path = join(dir, f.name);
        if (path === "SKILL.md") throw new Error("SKILL.md is written from the Instructions tab.");
        if (exists(path) && !confirm(`Replace ${path}?`)) continue;
        await api.putSkillFile(name(), path, f);
      }
      setFolders((l) => l.filter((d) => d !== dir));
    });
  }

  function newFolder() {
    const typed = prompt(`New folder${target() ? ` in ${target()}` : ""}:`);
    if (!typed) return;
    try {
      const path = join(target(), typed);
      setFolders((l) => [...l, path]);
      setTarget(path);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }

  function newFile() {
    const typed = prompt(`New file${target() ? ` in ${target()}` : ""}:`);
    if (!typed) return;
    void run(async () => {
      const path = join(target(), typed);
      if (exists(path)) throw new Error(`There is already a file ${path}.`);
      await api.putSkillFile(name(), path, new Blob([]));
      setFolders((l) => l.filter((d) => !path.startsWith(`${d}/`)));
      p.setOpen(path);
    });
  }

  function rename(path: string, folder: boolean) {
    if (path === "SKILL.md") return;
    const typed = prompt(`Rename ${path} to:`, path);
    if (!typed || typed.trim() === path) return;
    void run(async () => {
      const to = skillPath(typed);
      if (to === "SKILL.md") throw new Error("SKILL.md is written from the Instructions tab.");
      const moves = folder ? under(path).map((f) => [f.path, `${to}${f.path.slice(path.length)}`] as const) : [[path, to] as const];
      for (const [, dest] of moves) if (exists(dest)) throw new Error(`There is already a file ${dest}.`);
      for (const [from, dest] of moves) {
        const bytes = await api.skillFile(name(), from);
        await api.putSkillFile(name(), dest, bytes);
        await api.deleteSkillFile(name(), from);
      }
      if (folder) setFolders((l) => l.map((d) => (d === path || d.startsWith(`${path}/`) ? `${to}${d.slice(path.length)}` : d)));
      if (p.open && moves.some(([from]) => from === p.open)) p.setOpen(moves.find(([from]) => from === p.open)![1]);
    });
  }

  function remove(path: string, folder: boolean) {
    if (path === "SKILL.md") return;
    const files = folder ? under(path) : [];
    const q = folder ? `Delete the folder ${path} and the ${files.length} file${files.length === 1 ? "" : "s"} in it?` : `Delete ${path}?`;
    if (!confirm(q)) return;
    void run(async () => {
      for (const f of folder ? files.map((f) => f.path) : [path]) await api.deleteSkillFile(name(), f);
      if (folder) setFolders((l) => l.filter((d) => d !== path && !d.startsWith(`${path}/`)));
      if (p.open && (p.open === path || p.open.startsWith(`${path}/`))) p.setOpen(null);
      if (target() === path || target().startsWith(`${path}/`)) setTarget("");
    });
  }

  return (
    <section class="agent-tab skill-tab" aria-label="Files">
      <p class="muted">
        skills/{name()}/, {p.skill.files.length} file{p.skill.files.length === 1 ? "" : "s"}, {sizeText(p.skill.files.reduce((n, f) => n + f.size, 0))}.
        Scripts, references and assets the instructions name sit beside SKILL.md.
      </p>
      <Show when={!p.readOnly}>
        <div class="skill-files-bar">
          <label for={`${p.id}-folder`}>Folder</label>
          <select id={`${p.id}-folder`} value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
            <option value="">The skill's folder</option>
            <For each={allFolders()}>{(d) => <option value={d}>{d}</option>}</For>
          </select>
          <button type="button" disabled={busy()} onClick={() => upload.click()}>
            <TypeIcon name="upload" size={14} /> Upload files
          </button>
          <button type="button" disabled={busy()} onClick={newFile}>
            <TypeIcon name="file-plus" size={14} /> New file
          </button>
          <button type="button" disabled={busy()} onClick={newFolder}>
            <TypeIcon name="folder-plus" size={14} /> New folder
          </button>
          <input
            ref={upload}
            type="file"
            multiple
            class="visually-hidden"
            tabindex={-1}
            aria-label="Files to upload"
            onChange={(e) => {
              const files = [...(e.currentTarget.files ?? [])];
              e.currentTarget.value = "";
              if (files.length) uploaded(files);
            }}
          />
        </div>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="skill-files">
        <ul class="skill-tree" aria-label="Skill files">
          <For each={rows()}>
            {(r) => (
              <li class="skill-tree-row" classList={{ open: !r.folder && p.open === r.path }} style={{ "padding-left": `${r.depth * 1.1}rem` }}>
                <Show
                  when={!r.folder}
                  fallback={
                    <button type="button" class="skill-tree-name" title={r.path} onClick={() => setTarget(r.path)}>
                      <TypeIcon name="folder" size={14} /> {r.name}
                    </button>
                  }
                >
                  <button type="button" class="skill-tree-name" title={r.path} aria-current={p.open === r.path ? "true" : undefined} onClick={() => p.setOpen(r.path)}>
                    <TypeIcon name={r.path === "SKILL.md" ? SKILL_ICON : isTextFile(r.file!) ? "file-text" : "file"} size={14} /> {r.name}
                  </button>
                  <span class="muted skill-tree-size">{sizeText(r.file!.size)}</span>
                </Show>
                <Show when={!p.readOnly && r.path !== "SKILL.md"}>
                  <button type="button" class="skill-tree-action" aria-label={`Rename ${r.path}`} title="Rename" disabled={busy()} onClick={() => rename(r.path, r.folder)}>
                    <TypeIcon name="pencil" size={14} />
                  </button>
                  <button type="button" class="skill-tree-action danger" aria-label={`Delete ${r.path}`} title="Delete" disabled={busy()} onClick={() => remove(r.path, r.folder)}>
                    <TypeIcon name="trash-2" size={14} />
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
        <Show when={opened()} keyed>
          {(f) => <FileView skill={p.skill} file={f} readOnly={p.readOnly} reload={p.reload} />}
        </Show>
      </div>
    </section>
  );
}

/** An opened file: a text file in an editor with Save, anything else as its size and type. */
function FileView(props: { skill: Skill; file: SkillFile; readOnly: boolean; reload: () => Promise<void> }) {
  const text = isTextFile(props.file);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [value, setValue] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const isSkillMd = props.file.path === "SKILL.md";
  if (text) {
    api
      .skillFile(props.skill.name, props.file.path)
      .then((b) => b.text())
      .then((t) => {
        setSaved(t);
        setValue(t);
      })
      .catch((e: unknown) => setError(errorText(e)));
  }
  async function save() {
    const t = value();
    if (isSkillMd) await api.putSkill(props.skill.name, t);
    else await api.putSkillFile(props.skill.name, props.file.path, new Blob([t], { type: "text/plain" }));
    setSaved(t);
    await props.reload();
    void skillsChanged();
  }
  return (
    <div class="skill-file" role="region" aria-label={props.file.path}>
      <div class="skill-file-head">
        <strong>{props.file.path}</strong>
        <span class="muted">
          {sizeText(props.file.size)}, {props.file.media_type}
        </span>
        <a class="skill-download" href={skillFileUrl(props.skill.name, props.file.path)} download={props.file.path.split("/").pop()}>
          <TypeIcon name="download" size={14} /> Download
        </a>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show
        when={text}
        fallback={<p class="muted">This file is not text, so it is not shown here. Download it to look at it.</p>}
      >
        <Show when={saved() !== null} fallback={<p class="muted">Reading the file…</p>}>
          <Show when={isSkillMd && !props.readOnly}>
            <p class="muted">Saving SKILL.md here checks it as the Instructions tab does.</p>
          </Show>
          <textarea
            class="ai-system-prompt skill-body"
            aria-label={`Contents of ${props.file.path}`}
            rows={22}
            spellcheck={false}
            readOnly={props.readOnly}
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
          />
          <SaveBar dirty={value() !== saved()} readOnly={props.readOnly} save={save} />
        </Show>
      </Show>
    </div>
  );
}

/** The skill screen. */
export function SkillScreen() {
  const params = useParams();
  const navigate = useNavigate();
  const pane = usePane();
  const uid = createUniqueId();
  const [app, setApp] = createSignal<boolean | null>(null);
  void appInfo().then((a) => setApp(a !== null));
  const readOnly = () => app() !== false;
  const [skill, setSkill] = createSignal<Skill | null>(null);
  const [raw, setRaw] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [menu, setMenu] = createSignal(false);
  const [open, setOpen] = createSignal<string | null>(null);
  let more: HTMLButtonElement | undefined;

  async function load(name: string) {
    const [s, text] = await Promise.all([
      api.skill(name),
      api
        .skillFile(name, "SKILL.md")
        .then((b) => b.text())
        .catch(() => ""),
    ]);
    if (params.name !== name) return;
    setSkill(s);
    setRaw(text);
  }

  createEffect(
    on([() => params.name, skillsVersion], ([name]) => {
      setError(null);
      load(name!).catch((e: unknown) => {
        setSkill(null);
        setError(e instanceof ApiError && e.status === 404 ? `There is no skill called ${name}.` : errorText(e));
      });
    }),
  );
  createEffect(on(() => params.name, () => setOpen(null), { defer: true }));

  createEffect(() => {
    const s = skill();
    if (s) pane?.setTitle({ title: s.name, icon: s.valid ? SKILL_ICON : "triangle-alert" });
  });
  onCleanup(() => pane?.setTitle(null));

  const tab = () => TABS.find((t) => t.id === params.tab) ?? TABS[0];
  function go(id: string) {
    navigate(skillHref(params.name!, id), { replace: true });
  }
  function onTabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const i = TABS.findIndex((t) => t.id === tab().id);
    const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length]!;
    go(next.id);
    queueMicrotask(() => document.getElementById(`${uid}-tab-${next.id}`)?.focus());
  }

  async function remove() {
    setMenu(false);
    const name = params.name!;
    if (!confirm(`Delete the skill ${name}? Its folder is removed, and no agent is offered it.`)) return;
    try {
      await api.deleteSkill(name);
      await skillsChanged();
      navigate("/daily");
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <section class="list-page agent-screen skill-screen">
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={skill()}>
        {(s) => (
          <>
            <header class="note-header agent-header">
              <h1>
                <TypeIcon name={SKILL_ICON} size={22} /> {s().name}
              </h1>
              <Show when={!s().valid}>
                <span class="agent-badge agent-badge-invalid">Invalid</span>
              </Show>
              <div class="skill-more">
                <button
                  ref={more}
                  type="button"
                  class="pin-more"
                  aria-label={`Actions for ${s().name}`}
                  aria-haspopup="menu"
                  aria-expanded={menu()}
                  title="More actions"
                  onClick={() => setMenu(!menu())}
                >
                  <TypeIcon name="ellipsis" size={18} />
                </button>
                <Show when={menu()}>
                  <MenuBox label={`Actions for ${s().name}`} anchor={() => more} onClose={() => setMenu(false)}>
                    <a role="menuitem" href={skillExportUrl(s().name)} download={`${s().name}.zip`} onClick={() => setMenu(false)}>
                      Export
                    </a>
                    <Show when={!readOnly()}>
                      <button type="button" role="menuitem" class="danger" onClick={() => void remove()}>
                        Delete skill
                      </button>
                    </Show>
                  </MenuBox>
                </Show>
              </div>
            </header>
            <Show when={s().description}>
              <p class="muted">{s().description}</p>
            </Show>
            <Show when={!s().valid}>
              <p class="agent-readonly skill-invalid" role="note">
                This skill does not pass the Agent Skills checks, so no agent is offered it until it is fixed.
              </p>
            </Show>
            <Show when={readOnly()}>
              <p class="agent-readonly" role="note">
                Edit skills in the web app. This app can show them but not change them.
              </p>
            </Show>
            <div class="agent-tabs" role="tablist" aria-label="Skill" onKeyDown={onTabKey}>
              <For each={TABS}>
                {(t) => (
                  <button
                    type="button"
                    role="tab"
                    id={`${uid}-tab-${t.id}`}
                    aria-selected={tab().id === t.id}
                    aria-controls={`${uid}-panel`}
                    tabindex={tab().id === t.id ? 0 : -1}
                    onClick={() => go(t.id)}
                  >
                    {t.label}
                  </button>
                )}
              </For>
            </div>
            <div id={`${uid}-panel`} role="tabpanel" aria-labelledby={`${uid}-tab-${tab().id}`}>
              <Show
                when={tab().id === "files"}
                fallback={<InstructionsTab skill={s()} raw={raw()} reload={() => load(s().name)} readOnly={readOnly()} id={uid} />}
              >
                <FilesTab
                  skill={s()}
                  raw={raw()}
                  reload={() => load(s().name)}
                  readOnly={readOnly()}
                  id={uid}
                  open={open()}
                  setOpen={setOpen}
                />
              </Show>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}
