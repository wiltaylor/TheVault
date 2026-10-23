// The left sidebar's Agents section, below Types (agents G3, G6): a disclosure over every agent,
// Assistant first, each opening its agent screen in the active pane. In the web app a "+" creates
// one from a name and opens it, and each agent's ⋯ menu (or a right-click) opens, disables or
// enables, and deletes it. Assistant is built in: it has no Disable or Delete. In the apps the
// list is read only. An agent with a turn running on one of its threads shows a spinner, read as
// "Running" (agents G17).

import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { AgentSummary } from "../api";
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
import { errorText } from "../pages/TypeEditor";
import { isActiveLink } from "../workspace/links";
import { activePane } from "../workspace/model";
import { navigatePane, workspace } from "../workspace/store";
import { TypeIcon } from "./Icon";
import { NameInput } from "./PinnedTree";

const href = (name: string) => `/agents/${encodeURIComponent(name)}`;

export function AgentsNav(props: { onNavigate: () => void }) {
  const [menu, setMenu] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  onMount(() => void refreshAgents());

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
    let el!: HTMLDivElement;
    const close = (refocus: boolean) => {
      setMenu(null);
      if (refocus) p.anchor()?.focus();
    };
    const items = () => [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
    onMount(() => {
      items()[0]?.focus();
      const outside = (e: PointerEvent) => {
        if (!el.contains(e.target as Node) && !p.anchor()?.contains(e.target as Node)) close(false);
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
    const a = () => p.agent;
    const editable = () => !agentsReadOnly() && !a().built_in && !a().error;
    return (
      <div
        ref={el}
        class="pin-menu"
        role="menu"
        aria-label={`Actions for ${agentLabel(a())}`}
        onKeyDown={onKey}
        onPointerDown={(e) => e.stopPropagation()}
      >
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
      </div>
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
      </Show>
      <Show when={error() ?? agentsError()}>
        <p class="error pinned-error" role="alert">
          {error() ?? agentsError()}
        </p>
      </Show>
    </nav>
  );
}
