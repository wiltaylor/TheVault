// The left sidebar's Agents section, below Types (agents G3, G6): a disclosure over every agent,
// Assistant first, each opening its agent screen in the active pane. In the web app a "+" creates
// one from a name and opens it, and each agent's ⋯ menu (or a right-click) opens, disables or
// enables, and deletes it. Assistant is built in: it has no Disable or Delete. In the apps the
// list is read only. An agent with a turn running on one of its threads shows a spinner, read as
// "Running" (agents G17). Beneath the agents, a Skills subsection lists every skill by name, an
// invalid one with a warning mark, each opening its skill screen; in the web app its "+" menu
// makes a new skill from a name or imports one from a zip or a folder (skills S4).

import { For, type JSX, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { AgentSummary, SkillSummary } from "../api";
import {
  AGENT_ICON,
  agentLabel,
  agents,
  agentsError,
  agentsOpen,
  agentsReadOnly,
  createAgent,
  creatingAgent,
  deleteAgent,
  refreshAgents,
  setAgentEnabled,
  setAgentsOpen,
  setCreatingAgent,
} from "../agents";
import { RunningMark } from "../ai/RunningMark";
import {
  SKILL_ICON,
  canPickFolder,
  createSkill,
  creatingSkill,
  importSkillFolder,
  importSkillZip,
  refreshSkills,
  setCreatingSkill,
  skillHref,
  skills,
  skillsError,
} from "../skills";
import { errorText } from "../pages/TypeEditor";
import { isActiveLink } from "../workspace/links";
import { activePane } from "../workspace/model";
import { navigatePane, workspace } from "../workspace/store";
import { TypeIcon } from "./Icon";
import { NameInput } from "./PinnedTree";

const href = (name: string) => `/agents/${encodeURIComponent(name)}`;

/**
 * A popup menu beside the button that opened it: the first item takes focus, the arrows move
 * between items, Escape closes it and gives the button focus back, a press outside closes it.
 */
export function MenuBox(props: {
  label: string;
  anchor: () => HTMLElement | undefined;
  onClose: () => void;
  children: JSX.Element;
}) {
  let el!: HTMLDivElement;
  const close = (refocus: boolean) => {
    props.onClose();
    if (refocus) props.anchor()?.focus();
  };
  const items = () => [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
  onMount(() => {
    items()[0]?.focus();
    const outside = (e: PointerEvent) => {
      if (!el.contains(e.target as Node) && !props.anchor()?.contains(e.target as Node)) close(false);
    };
    document.addEventListener("pointerdown", outside, true);
    onCleanup(() => document.removeEventListener("pointerdown", outside, true));
  });
  function onKey(e: KeyboardEvent) {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      list[(at + (e.key === "ArrowDown" ? 1 : -1) + list.length) % list.length]?.focus();
    } else if (e.key === "Tab") {
      close(false);
    }
  }
  return (
    <div ref={el} class="pin-menu" role="menu" aria-label={props.label} onKeyDown={onKey} onPointerDown={(e) => e.stopPropagation()}>
      {props.children}
    </div>
  );
}

export function AgentsNav(props: { onNavigate: () => void }) {
  const [menu, setMenu] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  onMount(() => {
    void refreshAgents();
    void refreshSkills();
  });

  function open(name: string) {
    setMenu(null);
    navigatePane(activePane(workspace()), href(name));
    props.onNavigate();
  }

  function report(p: Promise<unknown>) {
    setError(null);
    p.catch((e: unknown) => setError(errorText(e)));
  }

  function create(text: string | null) {
    setCreatingAgent(false);
    if (!text) return;
    report(createAgent(text).then(open));
  }

  function Menu(p: { agent: AgentSummary; anchor: () => HTMLButtonElement | undefined }) {
    const a = () => p.agent;
    const editable = () => !agentsReadOnly() && !a().built_in && !a().error;
    return (
      <MenuBox label={`Actions for ${agentLabel(a())}`} anchor={p.anchor} onClose={() => setMenu(null)}>
        <button type="button" role="menuitem" onClick={() => open(a().name)}>
          Open
        </button>
        <Show when={editable()}>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenu(null);
              report(setAgentEnabled(a().name, a().enabled === false));
            }}
          >
            {a().enabled === false ? "Enable" : "Disable"}
          </button>
        </Show>
        <Show when={!agentsReadOnly() && !a().built_in}>
          <button
            type="button"
            role="menuitem"
            class="danger"
            onClick={() => {
              setMenu(null);
              if (!confirm(`Delete the agent ${agentLabel(a())}? Its file goes to the history; its threads stay.`)) return;
              report(deleteAgent(a().name));
            }}
          >
            Delete
          </button>
        </Show>
      </MenuBox>
    );
  }

  function Entry(p: { agent: AgentSummary }) {
    let more: HTMLButtonElement | undefined;
    const a = () => p.agent;
    const disabled = () => a().enabled === false;
    const title = () => {
      if (a().error) return `${agentLabel(a())}: ${a().error}`;
      return disabled() ? `${agentLabel(a())} (disabled)` : agentLabel(a());
    };
    return (
      <li>
        <div
          class="pin-row agent-row"
          classList={{ "menu-open": menu() === a().name, "agent-disabled": disabled(), "agent-broken": !!a().error }}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu(a().name);
          }}
        >
          <a
            href={href(a().name)}
            class="pin-note"
            classList={{ active: isActiveLink(href(a().name)) }}
            title={title()}
            onClick={() => props.onNavigate()}
          >
            <TypeIcon name={a().error ? "triangle-alert" : a().icon || AGENT_ICON} size={16} />
            <span>{agentLabel(a())}</span>
            <Show when={(a().running ?? 0) > 0}>
              <RunningMark />
            </Show>
            <Show when={disabled()}>
              <span class="visually-hidden"> (disabled)</span>
            </Show>
          </a>
          <button
            ref={more}
            type="button"
            class="pin-more"
            aria-label={`Actions for ${agentLabel(a())}`}
            aria-haspopup="menu"
            aria-expanded={menu() === a().name}
            title="More actions"
            onClick={() => setMenu(menu() === a().name ? null : a().name)}
          >
            <TypeIcon name="ellipsis" size={16} />
          </button>
          <Show when={menu() === a().name}>
            <Menu agent={a()} anchor={() => more} />
          </Show>
        </div>
      </li>
    );
  }

  return (
    <nav aria-label="Agents" class="types-nav agents-nav">
      <div class="types-nav-heading agents-nav-heading">
        <button
          type="button"
          class="types-nav-toggle"
          aria-expanded={agentsOpen()}
          aria-controls="agents-nav-list"
          onClick={() => setAgentsOpen(!agentsOpen())}
        >
          <span class="pin-chevron" classList={{ open: agentsOpen() }} aria-hidden="true">
            <TypeIcon name="chevron-right" size={14} />
          </span>
          <span>Agents</span>
        </button>
        <Show when={!agentsReadOnly()}>
          <button
            type="button"
            class="pinned-add"
            aria-label="New agent"
            title="New agent"
            onClick={() => {
              setAgentsOpen(true);
              setCreatingAgent(true);
            }}
          >
            <TypeIcon name="plus" size={16} />
          </button>
        </Show>
      </div>
      <Show when={agentsOpen()}>
        <Show when={creatingAgent() && !agentsReadOnly()}>
          <div class="pin-row pin-row-new">
            <TypeIcon name={AGENT_ICON} size={16} />
            <NameInput label="New agent name" onDone={create} />
          </div>
        </Show>
        <ul id="agents-nav-list" class="types-nav-list agents-nav-list">
          <For each={agents()}>{(a) => <Entry agent={a} />}</For>
        </ul>
        <SkillsNav onNavigate={props.onNavigate} report={report} />
      </Show>
      <Show when={error() ?? agentsError() ?? skillsError()}>
        <p class="error pinned-error" role="alert">
          {error() ?? agentsError() ?? skillsError()}
        </p>
      </Show>
    </nav>
  );
}

/** The Skills subsection beneath the agents (skills S4). */
function SkillsNav(props: { onNavigate: () => void; report: (p: Promise<unknown>) => void }) {
  const [menu, setMenu] = createSignal(false);
  let add: HTMLButtonElement | undefined;
  let zipInput!: HTMLInputElement;
  let folderInput!: HTMLInputElement;

  function open(name: string | null) {
    if (!name) return;
    navigatePane(activePane(workspace()), skillHref(name));
    props.onNavigate();
  }

  function create(text: string | null) {
    setCreatingSkill(false);
    if (!text) return;
    props.report(createSkill(text).then(open));
  }

  function picked(input: HTMLInputElement, run: (files: File[]) => Promise<string | null>) {
    const files = [...(input.files ?? [])];
    input.value = "";
    if (files.length) props.report(run(files).then(open));
  }

  function title(s: SkillSummary) {
    if (s.valid) return s.description ? `${s.name}: ${s.description}` : s.name;
    const errors = s.issues.filter((i) => i.severity === "error").map((i) => i.message);
    return `${s.name} is invalid${errors.length ? `: ${errors.join("; ")}` : ""}`;
  }

  return (
    <div class="skills-nav" role="group" aria-labelledby="skills-nav-heading">
      <div class="skills-nav-heading">
        <span id="skills-nav-heading">Skills</span>
        <Show when={!agentsReadOnly()}>
          <button
            ref={add}
            type="button"
            class="pinned-add"
            aria-label="Add skill"
            title="Add skill"
            aria-haspopup="menu"
            aria-expanded={menu()}
            onClick={() => setMenu(!menu())}
          >
            <TypeIcon name="plus" size={16} />
          </button>
          <Show when={menu()}>
            <MenuBox label="Add skill" anchor={() => add} onClose={() => setMenu(false)}>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  setCreatingSkill(true);
                }}
              >
                New skill
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  zipInput.click();
                }}
              >
                Import skill from a zip
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
                  Import skill from a folder
                </button>
              </Show>
            </MenuBox>
          </Show>
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
      <Show when={creatingSkill() && !agentsReadOnly()}>
        <div class="pin-row pin-row-new">
          <TypeIcon name={SKILL_ICON} size={16} />
          <NameInput label="New skill name" onDone={create} />
        </div>
      </Show>
      <ul class="types-nav-list agents-nav-list skills-nav-list">
        <For each={skills()}>
          {(s) => (
            <li>
              <div class="pin-row agent-row" classList={{ "agent-broken": !s.valid }}>
                <a
                  href={skillHref(s.name)}
                  class="pin-note"
                  classList={{ active: isActiveLink(skillHref(s.name)) }}
                  title={title(s)}
                  onClick={() => props.onNavigate()}
                >
                  <TypeIcon name={s.valid ? SKILL_ICON : "triangle-alert"} size={16} />
                  <span>{s.name}</span>
                  <Show when={!s.valid}>
                    <span class="visually-hidden"> (invalid)</span>
                  </Show>
                </a>
              </div>
            </li>
          )}
        </For>
      </ul>
      <Show when={skills().length === 0 && !creatingSkill()}>
        <p class="muted skills-nav-empty">No skills yet.</p>
      </Show>
    </div>
  );
}
