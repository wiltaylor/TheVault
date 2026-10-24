// The agent screen's Memory tab (agents G21): the agent's memories, newest first, with a search
// box; Add, Edit (title and text, one Save) and Delete (to the trash, after asking). The changes
// are note writes authored by the user through `/api/agents/{name}/memories`, so the tab works in
// the apps too, where they go to the device's vault and sync like any note.

import { For, Show, createEffect, createResource, createSignal, on } from "solid-js";
import { memoriesVersion } from "../agents";
import { type AgentMemory, api } from "../api";
import { Spinner } from "../components/SyncStatus";
import { errorText } from "./TypeEditor";

/** The longest memory title and text, in characters, as the server holds them. */
export const MEMORY_TITLE_MAX = 120;
export const MEMORY_TEXT_MAX = 8000;

/** What the editor holds: a new memory (`id` null) or a change to one. */
interface Draft {
  id: string | null;
  base: string | null;
  title: string;
  text: string;
}

export function MemoryTab(props: { agent: string; label: string; id: string }) {
  const id = (what: string) => `${props.id}-memory-${what}`;
  const [query, setQuery] = createSignal("");
  const [reload, setReload] = createSignal(0);
  const [list] = createResource(
    () => ({ agent: props.agent, query: query(), n: reload(), v: memoriesVersion() }),
    (k) =>
      api
        .agentMemories(k.agent, k.query)
        .then((l) => ({ ...l, error: null as string | null }))
        .catch((e: unknown) => ({ memories: [] as AgentMemory[], held: 0, limit: 0, error: errorText(e) })),
  );
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);

  // Another agent's screen starts afresh.
  createEffect(
    on(
      () => props.agent,
      () => {
        setDraft(null);
        setQuery("");
        setError(null);
        setNotice(null);
      },
      { defer: true },
    ),
  );

  function edit(m: AgentMemory | null) {
    setError(null);
    setNotice(null);
    setDraft(m ? { id: m.id, base: m.version, title: m.title, text: m.text } : { id: null, base: null, title: "", text: "" });
  }

  const unchanged = () => {
    const d = draft();
    const m = d?.id ? list()?.memories.find((x) => x.id === d.id) : undefined;
    return !!m && m.title === d!.title && m.text === d!.text;
  };

  async function save() {
    const d = draft();
    if (!d) return;
    setError(null);
    setSaving(true);
    try {
      if (d.id) await api.updateAgentMemory(props.agent, d.id, { title: d.title, text: d.text, base_version: d.base ?? undefined });
      else await api.createAgentMemory(props.agent, d.title, d.text);
      setDraft(null);
      setNotice(d.id ? "Saved." : "Memory added.");
      setReload((n) => n + 1);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  async function remove(m: AgentMemory) {
    if (!confirm(`Delete the memory "${m.title}"? It moves to the trash, where you can restore it.`)) return;
    setError(null);
    setNotice(null);
    try {
      await api.deleteAgentMemory(props.agent, m.id);
      if (draft()?.id === m.id) setDraft(null);
      setNotice("Memory moved to the trash.");
      setReload((n) => n + 1);
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <section class="agent-tab agent-memory" aria-label="Memory">
      <p class="muted">
        What {props.label} remembers from one conversation to the next. The start of each memory goes into its instructions
        every turn. It adds and changes these itself; you can too.
      </p>
      <div class="agent-memory-bar">
        <input
          type="search"
          aria-label="Search memories"
          placeholder="Search memories"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <button type="button" class="primary" disabled={draft()?.id === null} onClick={() => edit(null)}>
          Add memory
        </button>
        <Show when={list() && !list()!.error && list()}>
          {(l) => (
            <span class="muted">
              {l().held} of {l().limit} memories
            </span>
          )}
        </Show>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={notice()}>
        <p class="muted" role="status">
          {notice()}
        </p>
      </Show>
      <Show when={draft()}>
        {(d) => (
          <form
            class="agent-memory-editor"
            aria-label={d().id ? "Edit memory" : "New memory"}
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <label for={id("title")}>Title</label>
            <input
              id={id("title")}
              maxLength={MEMORY_TITLE_MAX}
              value={d().title}
              onInput={(e) => {
                const title = e.currentTarget.value;
                setDraft((x) => (x ? { ...x, title } : x));
              }}
            />
            <label for={id("text")}>Text</label>
            <textarea
              id={id("text")}
              class="ai-system-prompt"
              rows={6}
              maxLength={MEMORY_TEXT_MAX}
              value={d().text}
              onInput={(e) => {
                const text = e.currentTarget.value;
                setDraft((x) => (x ? { ...x, text } : x));
              }}
            />
            <p class="muted">
              {[...d().text].length} / {MEMORY_TEXT_MAX} characters
            </p>
            <div class="actions">
              <button
                type="submit"
                class="primary"
                disabled={saving() || !d().title.trim() || !d().text.trim() || unchanged()}
                aria-busy={saving() ? "true" : undefined}
              >
                <Show when={saving()}>
                  <Spinner />{" "}
                </Show>
                Save
              </button>
              <button type="button" disabled={saving()} onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </Show>
      <Show when={list()?.error}>
        <p class="error" role="alert">
          {list()!.error}
        </p>
      </Show>
      <Show when={list() && !list()!.error && list()}>
        {(l) => (
          <Show
            when={l().memories.length > 0}
            fallback={
              <p class="muted">
                {query().trim() ? "No memory matches the search." : `${props.label} has no memories yet.`}
              </p>
            }
          >
            <ul class="agent-memory-list">
              <For each={l().memories}>
                {(m) => (
                  <li class="agent-memory-item">
                    <div class="agent-memory-head">
                      <strong>{m.title}</strong>
                      <Show when={m.updated}>
                        <time class="muted" datetime={m.updated!}>
                          {new Date(m.updated!).toLocaleString()}
                        </time>
                      </Show>
                    </div>
                    <p class="agent-memory-text">{m.text}</p>
                    <div class="actions">
                      <button type="button" aria-label={`Edit ${m.title}`} onClick={() => edit(m)}>
                        Edit
                      </button>
                      <button type="button" class="danger" aria-label={`Delete ${m.title}`} onClick={() => void remove(m)}>
                        Delete
                      </button>
                    </div>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        )}
      </Show>
    </section>
  );
}
