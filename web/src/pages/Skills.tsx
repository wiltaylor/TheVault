// The Skills page, `/skills` (skills S4): every skill as a card, with its name opening the skill
// screen, its description cut to two lines, its file count and size, its license, and an invalid
// skill's first issue. A search box narrows the list. The header has New skill, which asks for a
// name and opens the new skill, and in the web app Import, from a zip or (where the browser can
// choose one) a folder, asking before it replaces a skill of the same name. The apps write a new
// skill on the home server, which takes the device's token for it; an import needs `admin`,
// which a device token never has (skills S3).

import { useNavigate } from "@solidjs/router";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { SkillSummary } from "../api";
import { MenuBox } from "../components/AgentsNav";
import { SearchIcon, TypeIcon } from "../components/Icon";
import { NameInput } from "../components/PinnedTree";
import { appInfo } from "../inApp";
import {
  SKILLS_DOC,
  SKILL_ICON,
  canPickFolder,
  createSkill,
  creatingSkill,
  importSkillFolder,
  importSkillZip,
  refreshSkills,
  setCreatingSkill,
  sizeText,
  skillHref,
  skillMatches,
  skills,
  skillsError,
} from "../skills";
import { errorText } from "./TypeEditor";

/** The issue an invalid skill's card shows: its first error, else its first issue. */
export function firstIssue(s: SkillSummary): string | null {
  const i = s.issues.find((x) => x.severity === "error") ?? s.issues[0];
  return i ? `${i.at ? `${i.at}: ` : ""}${i.message}` : null;
}

export function Skills() {
  const navigate = useNavigate();
  const [app, setApp] = createSignal<boolean | null>(null);
  void appInfo().then((a) => setApp(a !== null));
  // In the apps: Import is the web app's.
  const noImport = () => app() !== false;
  const [query, setQuery] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [menu, setMenu] = createSignal(false);
  let importButton: HTMLButtonElement | undefined;
  let zipInput!: HTMLInputElement;
  let folderInput!: HTMLInputElement;

  onMount(() => void refreshSkills());
  onCleanup(() => setCreatingSkill(false));

  const shown = createMemo(() => skills().filter((s) => skillMatches(s, query())));

  function open(name: string | null) {
    if (name) navigate(skillHref(name));
  }

  function report(p: Promise<unknown>) {
    setError(null);
    p.catch((e: unknown) => setError(errorText(e)));
  }

  function create(text: string | null) {
    setCreatingSkill(false);
    if (!text) return;
    report(createSkill(text).then(open));
  }

  function picked(input: HTMLInputElement, run: (files: File[]) => Promise<string | null>) {
    const files = [...(input.files ?? [])];
    input.value = "";
    if (files.length) report(run(files).then(open));
  }

  return (
    <section class="list-page skills-page">
      <header class="note-header">
        <h1 class="type-list-title">
          <TypeIcon name={SKILL_ICON} size={24} />
          Skills
        </h1>
        <div class="actions skills-actions">
          <button type="button" class="button primary" onClick={() => setCreatingSkill(true)}>
            New skill
          </button>
          <Show when={!noImport()}>
            <div class="skill-more">
              <button
                ref={importButton}
                type="button"
                class="button"
                aria-haspopup="menu"
                aria-expanded={menu()}
                onClick={() => setMenu(!menu())}
              >
                Import
              </button>
              <Show when={menu()}>
                <MenuBox label="Import skill" anchor={() => importButton} onClose={() => setMenu(false)}>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenu(false);
                      zipInput.click();
                    }}
                  >
                    From a zip
                  </button>
                  <Show when={canPickFolder()}>
                    <button
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        setMenu(false);
                        folderInput.click();
                      }}
                    >
                      From a folder
                    </button>
                  </Show>
                </MenuBox>
              </Show>
            </div>
            <input
              ref={zipInput}
              type="file"
              accept=".zip,application/zip"
              class="visually-hidden"
              tabindex={-1}
              aria-label="Skill zip"
              onChange={(e) => picked(e.currentTarget, ([f]) => importSkillZip(f!, f!.name))}
            />
            <input
              ref={(el) => {
                folderInput = el;
                el.setAttribute("webkitdirectory", "");
              }}
              type="file"
              multiple
              class="visually-hidden"
              tabindex={-1}
              aria-label="Skill folder"
              onChange={(e) => picked(e.currentTarget, (files) => importSkillFolder(files))}
            />
          </Show>
        </div>
      </header>

      <Show when={creatingSkill()}>
        <div class="skills-new">
          <TypeIcon name={SKILL_ICON} size={16} />
          <NameInput label="New skill name" onDone={create} />
        </div>
      </Show>

      <Show when={error() ?? skillsError()}>
        <p class="error" role="alert">
          {error() ?? skillsError()}
        </p>
      </Show>

      <Show
        when={skills().length > 0}
        fallback={
          <Show when={!creatingSkill()}>
            <div class="skills-empty">
              <p>
                No skills yet. A skill is a folder of instructions, with any scripts, references and assets they need, that an
                agent follows when a task calls for it. Agents choose a skill by its description.
              </p>
              <p class="muted">
                {noImport()
                  ? "Use New skill to write one. Import one from a zip or a folder in the web app."
                  : "Use New skill to write one, or Import to add one from a zip or a folder."}
              </p>
              <p>
                <a href={SKILLS_DOC} target="_blank" rel="noopener noreferrer">
                  How skills work
                </a>
              </p>
            </div>
          </Show>
        }
      >
        <div class="type-list-toolbar">
          <div class="search-bar type-list-search" role="search">
            <SearchIcon />
            <input
              type="search"
              placeholder="Search skills"
              aria-label="Search skills"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape" && query()) {
                  e.preventDefault();
                  setQuery("");
                }
              }}
            />
          </div>
        </div>
        <Show when={shown().length > 0} fallback={<p class="muted">No skills match.</p>}>
          <ul class="skills-list" aria-label="Skills">
            <For each={shown()}>
              {(s) => (
                <li class="skill-card" classList={{ "skill-card-invalid": !s.valid }}>
                  <div class="skill-card-head">
                    <TypeIcon name={s.valid ? SKILL_ICON : "triangle-alert"} size={16} />
                    <a href={skillHref(s.name)} class="skill-card-name">
                      {s.name}
                    </a>
                    <Show when={!s.valid}>
                      <span class="agent-badge agent-badge-invalid">Invalid</span>
                    </Show>
                  </div>
                  <Show when={s.description}>
                    <p class="skill-card-description" title={s.description ?? undefined}>
                      {s.description}
                    </p>
                  </Show>
                  <Show when={!s.valid && firstIssue(s)}>
                    <p class="error skill-card-issue">{firstIssue(s)}</p>
                  </Show>
                  <p class="muted skill-card-meta">
                    {s.files} file{s.files === 1 ? "" : "s"} · {sizeText(s.size)}
                    <Show when={s.license}> · {s.license}</Show>
                  </p>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </section>
  );
}
