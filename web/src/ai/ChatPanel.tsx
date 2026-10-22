// The right sidebar's AI panel (ai D14, D16): ask about the vault, watch the answer stream, see
// which notes the model searched and read, and apply or discard the edits it proposes. The
// conversation lives in this tab's memory (store.ts); New conversation forgets it.

import { For, Match, Show, Switch, createSignal, onMount } from "solid-js";
import { type AiModel, type AiProvider, type AiStatus, type Json, api, assetUrl } from "../api";
import { formatValue } from "../components/Render";
import { blockLines, lineDiff } from "./diff";
import { Markdown } from "./markdown";
import {
  type Part,
  type Proposal,
  applyProposal,
  ask,
  chat,
  discardProposal,
  newConversation,
  providerLabel,
  setModel,
  setProvider,
  stop,
} from "./store";
import "./chat.css";

/** What a tool step says it did. */
export function stepLabel(name: string, input: Json): string {
  const o = input && typeof input === "object" && !Array.isArray(input) ? input : {};
  const s = (k: string) => (typeof o[k] === "string" ? (o[k] as string) : "");
  switch (name) {
    case "search_notes":
      return `Searched for “${s("text")}”`;
    case "query_notes":
      return `Queried ${s("query")}`;
    case "get_note":
      return "Read a note";
    case "backlinks":
      return "Looked up backlinks";
    case "read_asset":
      return "Read a file";
    case "propose_edit":
      return "Checked an edit";
    default:
      return `Called ${name}`;
  }
}

function ToolStep(props: { part: Extract<Part, { kind: "tool" }> }) {
  const status = () => {
    if (!props.part.done) return "running";
    if (props.part.isError) return "failed";
    if (props.part.asset) return props.part.asset.filename;
    return `${props.part.notes.length} found`;
  };
  // The image a read_asset step read, from the asset's own URL: the stream never carries its bytes (ai D19).
  const image = () => {
    const a = props.part.asset;
    return a && a.media_type.startsWith("image/") ? a : undefined;
  };
  return (
    <details class="ai-step" classList={{ "ai-step-error": props.part.isError }}>
      <summary>
        <span>{stepLabel(props.part.name, props.part.input)}</span> <span class="muted">({status()})</span>
        <Show when={image()}>
          {(a) => (
            <a class="ai-step-thumb" href={assetUrl(a().id)} target="_blank" rel="noopener">
              <img src={assetUrl(a().id)} alt={a().filename} loading="lazy" />
            </a>
          )}
        </Show>
      </summary>
      <Show when={props.part.message}>
        <p class="muted">{props.part.message}</p>
      </Show>
      <Show when={props.part.notes.length}>
        <ul>
          <For each={props.part.notes}>
            {(n) => (
              <li>
                <a href={`/notes/${encodeURIComponent(n.id)}`}>{n.title || n.id}</a>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </details>
  );
}

function BlockDiff(props: { before: Json; after: Json }) {
  const lines = () => lineDiff(blockLines(props.before), blockLines(props.after));
  return (
    <pre class="ai-diff-lines">
      <For each={lines()}>
        {(l) => (
          <Switch fallback={<span class="ai-same">{`  ${l.text}\n`}</span>}>
            <Match when={l.type === "del"}>
              <del>{`- ${l.text}\n`}</del>
            </Match>
            <Match when={l.type === "add"}>
              <ins>{`+ ${l.text}\n`}</ins>
            </Match>
          </Switch>
        )}
      </For>
    </pre>
  );
}

function ProposalCard(props: { part: Extract<Part, { kind: "proposal" }>; turn: number; index: number }) {
  const p = (): Proposal => props.part.proposal;
  const title = () => (typeof p().title === "string" && p().title ? (p().title as string) : p().note_id);
  return (
    <section class="ai-proposal" aria-label={`Proposed edit to ${title()}`}>
      <p class="ai-proposal-head">
        Proposed edit to <a href={`/notes/${encodeURIComponent(p().note_id)}`}>{title()}</a>
      </p>
      <ul class="ai-diff">
        <For each={p().ops}>
          {(op) => (
            <li>
              <span class="ai-diff-name">{op.op === "field" ? op.name : op.name.replace(/^content /, "")}</span>
              <Show when={op.op === "field"} fallback={<BlockDiff before={op.before} after={op.after} />}>
                <div class="ai-diff-field">
                  <Show when={op.before !== null}>
                    <del>{formatValue(op.before)}</del>
                  </Show>
                  <Show when={op.after !== null}>
                    <ins>{formatValue(op.after)}</ins>
                  </Show>
                </div>
              </Show>
            </li>
          )}
        </For>
      </ul>
      <Switch>
        <Match when={props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying"}>
          <Show when={props.part.error}>
            <p role="alert" class="ai-error">
              {props.part.error}
            </p>
          </Show>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void applyProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => discardProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.
          </p>
        </Match>
        <Match when={props.part.state === "discarded"}>
          <p role="status" class="muted">
            Discarded.
          </p>
        </Match>
      </Switch>
    </section>
  );
}

function Notice(props: { part: Extract<Part, { kind: "notice" }> }) {
  return (
    <p
      class={`ai-notice ai-notice-${props.part.notice}`}
      role={props.part.notice === "refusal" || props.part.notice === "error" ? "alert" : "status"}
    >
      {props.part.text}
    </p>
  );
}

/** The providers in the order Settings > AI shows them. */
const PROVIDERS: AiProvider[] = ["anthropic", "openai", "chatgpt"];

/**
 * The provider and model picker: the providers Settings > AI has set up, and each one's live
 * model list (ai D6), starting from the defaults chosen there.
 */
function Picker() {
  const [status, setStatus] = createSignal<AiStatus | null>(null);
  const [models, setModels] = createSignal<AiModel[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const configured = () => PROVIDERS.filter((p) => status()?.providers[p]?.configured);
  async function loadModels(p: AiProvider) {
    setProvider(p, chat.provider === p ? chat.model : (status()?.providers[p]?.model ?? null));
    try {
      const got = await api.aiModels(p);
      if (chat.provider !== p) return;
      setModels(got.models);
      setError(null);
      if (!chat.model || !got.models.some((m) => m.id === chat.model)) setModel(got.model);
    } catch (e) {
      if (chat.provider !== p) return;
      setModels([]);
      setError(e instanceof Error ? e.message : String(e));
    }
  }
  onMount(async () => {
    try {
      const s = await api.aiStatus();
      setStatus(s);
      const ready = configured();
      const pick = [chat.provider, s.default_provider].find((p): p is AiProvider => !!p && ready.includes(p as AiProvider)) ?? ready[0];
      if (pick) await loadModels(pick);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  });
  return (
    <div class="ai-picker">
      <Show when={status() && configured().length === 0}>
        <p class="muted">
          No AI provider is set up. Add one in <a href="/settings/ai">Settings &gt; AI</a>.
        </p>
      </Show>
      <Show when={configured().length > 0}>
        <select aria-label="Provider" value={chat.provider ?? ""} onChange={(e) => void loadModels(e.currentTarget.value as AiProvider)}>
          <For each={PROVIDERS}>
            {(p) => (
              <option value={p} disabled={!configured().includes(p)}>
                {providerLabel(p)}
              </option>
            )}
          </For>
        </select>
        <select aria-label="Model" value={chat.model ?? ""} onChange={(e) => setModel(e.currentTarget.value || null)}>
          <Show when={!chat.model}>
            <option value="">Choose a model</option>
          </Show>
          <Show when={chat.model && !models().some((m) => m.id === chat.model)}>
            <option value={chat.model!}>{chat.model}</option>
          </Show>
          <For each={models()}>{(m) => <option value={m.id}>{m.display_name || m.id}</option>}</For>
        </select>
      </Show>
      <Show when={error()}>
        <p class="ai-error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}

/** The AI panel. `noteId` is the note open in the active pane, if any. */
export function ChatPanel(props: { noteId: () => string }) {
  const [draft, setDraft] = createSignal("");
  let list: HTMLOListElement | undefined;
  function send() {
    const text = draft();
    if (!text.trim() || chat.streaming) return;
    setDraft("");
    void ask(text, props.noteId() || undefined);
    queueMicrotask(() => list?.lastElementChild?.scrollIntoView?.({ block: "end" }));
  }
  return (
    <div class="ai-panel">
      <div class="ai-toolbar">
        <Picker />
        <button type="button" class="link" onClick={newConversation} disabled={chat.turns.length === 0 && !chat.streaming}>
          New conversation
        </button>
      </div>
      <Show when={chat.turns.length === 0}>
        <p class="muted ai-empty">
          Ask about your notes. Answers come from {providerLabel(chat.provider)}; the notes it reads are sent there. Edits are proposals you apply.
        </p>
      </Show>
      <ol class="ai-turns" ref={list} aria-label="Conversation" aria-busy={chat.streaming}>
        <For each={chat.turns}>
          {(turn, t) => (
            <li class={`ai-turn ai-turn-${turn.role}`}>
              <Show when={turn.role === "user"} fallback={
                <For each={turn.parts}>
                  {(part, i) => (
                    <Switch>
                      <Match when={part.kind === "text" && part}>{(p) => <Markdown text={p().text} />}</Match>
                      <Match when={part.kind === "tool" && part}>{(p) => <ToolStep part={p()} />}</Match>
                      <Match when={part.kind === "proposal" && part}>{(p) => <ProposalCard part={p()} turn={t()} index={i()} />}</Match>
                      <Match when={part.kind === "notice" && part}>{(p) => <Notice part={p()} />}</Match>
                    </Switch>
                  )}
                </For>
              }>
                <p class="ai-question">{turn.text}</p>
              </Show>
            </li>
          )}
        </For>
      </ol>
      <form
        class="ai-form"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          aria-label="Ask AI"
          placeholder="Ask about your notes"
          rows={3}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
              e.preventDefault();
              send();
            }
          }}
        />
        <Show
          when={chat.streaming}
          fallback={
            <button type="submit" class="primary" disabled={!draft().trim()}>
              Send
            </button>
          }
        >
          <button type="button" onClick={stop}>
            Stop
          </button>
        </Show>
      </form>
    </div>
  );
}
