// The right sidebar's AI panel (ai D14, D23): ask about the vault, watch the answer stream, see
// which notes the model searched and read, and apply or discard the edits it proposes. The
// conversation lives in this tab's memory (store.ts); New conversation forgets it. It shows in
// the web app and in the desktop and Android apps, whose chat runs on the home server.

import { For, Match, Show, Switch, createSignal, onMount } from "solid-js";
import { type AiModel, type AiProvider, type AiProviders, type Json, type NoteDoc, api, assetUrl } from "../api";
import { formatValue } from "../components/Render";
import { blockLines, lineDiff } from "./diff";
import { DownloadCard } from "./DownloadCard";
import { Markdown } from "./markdown";
import { TypeProposalCard } from "./TypeProposalCard";
import {
  type Applied,
  type NoteOp,
  type Part,
  type Proposal,
  type ProposalOp,
  applyAll,
  applyProposal,
  ask,
  chat,
  discardProposal,
  inApp,
  newConversation,
  pending,
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
    case "find_assets":
      return s("text").trim() ? `Looked for files named “${s("text").trim()}”` : "Listed the newest files";
    case "propose_edit":
      return "Checked an edit";
    case "propose_create":
      return "Checked a new note";
    case "propose_archive":
      return o.archived === false ? "Checked an unarchive" : "Checked an archive";
    case "propose_delete":
      return "Checked a delete";
    case "list_types":
      return "Listed the types";
    case "get_type":
      return `Read type ${s("name")}`;
    case "type_format":
      return "Read the type file format";
    case "propose_type_change":
      return `Checked a change to type ${s("name")}`;
    case "web_search":
      return `Searched the web for “${s("query")}”`;
    case "fetch_url":
      return `Read ${hostOf(s("url"))}`;
    case "download_asset":
      return `Download from ${hostOf(s("url"))}`;
    default:
      return `Called ${name}`;
  }
}

/** The type tools (ai D18), whose steps return no notes. */
const TYPE_TOOLS = new Set(["list_types", "get_type", "type_format", "propose_type_change"]);

/** The host of `url`, or `url` itself when it is not a URL. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** The web tools (ai D27): their steps name the address read, linking out. */
const WEB_TOOLS: Record<string, string> = { fetch_url: "Read", download_asset: "Download from" };

/** An http or https URL, safe to link to. */
function webLink(url: string): string | undefined {
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : undefined;
  } catch {
    return undefined;
  }
}

/** A web step's words: what it did, and the host linking to the URL (ai D27). */
function WebLabel(props: { name: string; input: Json }) {
  const url = () => {
    const o = props.input && typeof props.input === "object" && !Array.isArray(props.input) ? props.input : {};
    return typeof o.url === "string" ? o.url : "";
  };
  return (
    <span>
      {WEB_TOOLS[props.name]}{" "}
      <Show when={webLink(url())} fallback={hostOf(url())}>
        {(href) => (
          <a href={href()} target="_blank" rel="noopener noreferrer">
            {hostOf(url())}
          </a>
        )}
      </Show>
    </span>
  );
}

function ToolStep(props: { part: Extract<Part, { kind: "tool" }> }) {
  const status = () => {
    if (!props.part.done) return "running";
    if (props.part.isError) return "failed";
    if (props.part.asset) return props.part.asset.filename;
    if (props.part.web) return props.part.web.title || props.part.web.media_type || "done";
    if (props.part.name in WEB_TOOLS) return "done";
    if (TYPE_TOOLS.has(props.part.name)) return "done";
    if (props.part.links) return `${props.part.links.length} found`;
    if (props.part.assets) return `${props.part.assets.length} found`;
    return `${props.part.notes.length} found`;
  };
  // The image a read_asset step read, from the asset's own URL: the stream never carries its bytes (ai D19).
  // In an app an asset not on the device does not load: its file name is shown alone.
  const [broken, setBroken] = createSignal(false);
  const image = () => {
    const a = props.part.asset;
    return a && a.media_type.startsWith("image/") && !broken() ? a : undefined;
  };
  return (
    <details class="ai-step" classList={{ "ai-step-error": props.part.isError }}>
      <summary>
        <Show when={props.part.name in WEB_TOOLS} fallback={<span>{stepLabel(props.part.name, props.part.input)}</span>}>
          <WebLabel name={props.part.name} input={props.part.input} />
        </Show>{" "}
        <span class="muted">({status()})</span>
        <Show when={image()}>
          {(a) => (
            <a class="ai-step-thumb" href={assetUrl(a().id)} target="_blank" rel="noopener">
              <img src={assetUrl(a().id)} alt={a().filename} loading="lazy" onError={() => setBroken(true)} />
            </a>
          )}
        </Show>
      </summary>
      <Show when={props.part.message}>
        <p class="muted">{props.part.message}</p>
      </Show>
      {/* A web_search step's results link out, in a new tab (ai D26). */}
      <Show when={props.part.links?.length}>
        <ul>
          <For each={props.part.links}>
            {(l) => (
              <li>
                <a href={l.url} target="_blank" rel="noopener noreferrer">
                  {l.title}
                </a>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={props.part.web && webLink(props.part.web.url)}>
        {(href) => (
          <p class="muted">
            <a href={href()} target="_blank" rel="noopener noreferrer">
              {href()}
            </a>
          </p>
        )}
      </Show>
      <Show when={props.part.assets?.length}>
        <ul>
          <For each={props.part.assets}>
            {(a) => (
              <li>
                <a href={assetUrl(a.id)} target="_blank" rel="noopener">
                  {a.filename}
                </a>
              </li>
            )}
          </For>
        </ul>
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

/** A card's heading words, before the note's title, by what the proposal does (ai D20). */
const HEADS: Record<NoteOp, string> = {
  update: "Proposed edit to",
  create: "Proposed new note",
  archive: "Proposed archive of",
  unarchive: "Proposed unarchive of",
  delete: "Proposed delete of",
};

/** What an applied card says. */
const APPLIED: Record<NoteOp, string> = {
  update: "Applied.",
  create: "Created.",
  archive: "Archived.",
  unarchive: "Unarchived.",
  delete: "Moved to the trash.",
};

/** Fields the server sets itself, left out of a new note's card as the server leaves them out of a diff. */
const SERVER_FIELDS = new Set(["created", "updated"]);

/** A new note's changes for its card: every field and top-level block it sets, as additions. */
export function createOps(after: NoteDoc | undefined): ProposalOp[] {
  if (!after) return [];
  const fields = Object.entries(after.fields ?? {})
    .filter(([k, v]) => !SERVER_FIELDS.has(k) && v !== null && v !== "" && !(Array.isArray(v) && v.length === 0))
    .map(([name, v]): ProposalOp => ({ op: "field", name, before: null, after: v }));
  const blocks = (after.blocks ?? []).map(
    (b): ProposalOp => ({ op: "block", name: [b.kind, ...(b.labels ?? [])].join(" "), before: null, after: b as unknown as Json }),
  );
  return [...fields, ...blocks];
}

function ProposalCard(props: { part: Extract<Part, { kind: "proposal" }>; turn: number; index: number }) {
  const p = (): Proposal => props.part.proposal;
  const noteId = () => {
    const x = p();
    return "note_id" in x ? x.note_id : undefined;
  };
  const title = () => (typeof p().title === "string" && p().title ? (p().title as string) : (noteId() ?? "Untitled"));
  const ops = (): ProposalOp[] => {
    const x = p();
    if (x.op === "update") return x.ops;
    if (x.op === "create") return createOps(x.after);
    return [];
  };
  const link = () => noteId() ?? props.part.created;
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${HEADS[p().op]} ${title()}`}>
      <p class="ai-proposal-head">
        {HEADS[p().op]}{" "}
        <Show when={link()} fallback={<strong>{title()}</strong>}>
          {(id) => <a href={`/notes/${encodeURIComponent(id())}`}>{title()}</a>}
        </Show>
        <Show when={p().op === "create" && "type" in p()}>
          <span class="muted"> ({(p() as { type: string }).type})</span>
        </Show>
      </p>
      <Show when={p().op === "delete"}>
        <p class="muted">Applied, it moves the note to the trash, where you can restore it.</p>
      </Show>
      <ul class="ai-diff" hidden={ops().length === 0}>
        <For each={ops()}>
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
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void applyProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => discardProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => discardProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            {APPLIED[p().op]}
            <Show when={p().op === "create" && props.part.created}>
              {(id) => (
                <>
                  {" "}
                  <a href={`/notes/${encodeURIComponent(id())}`}>Open the new note</a>
                </>
              )}
            </Show>
            <Show when={p().op === "delete"}>
              {" "}
              <a href="/trash">Open the trash</a>
            </Show>
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

/** An applied card's words before the note's title, by what the AI did (ai D22). */
const DID: Record<NoteOp, string> = {
  update: "Edited",
  create: "Created",
  archive: "Archived",
  unarchive: "Unarchived",
  delete: "Moved",
};

type AppliedNote = Extract<Applied, { note_id: string }>;

/**
 * A change the AI made itself, its confirmation off in Settings > AI (ai D22): what it did,
 * linking to the note, the trash or the type.
 */
export function AppliedCard(props: { applied: Applied }) {
  const a = () => props.applied;
  const note = (): AppliedNote | undefined => {
    const x = a();
    return "note_id" in x ? x : undefined;
  };
  const typeName = () => {
    const x = a();
    return "type_name" in x ? x.type_name : "";
  };
  const title = (n: { title: Json; note_id: string }) => (typeof n.title === "string" && n.title ? n.title : n.note_id);
  const label = () => {
    const n = note();
    if (!n) return `${a().op === "type_create" ? "Created" : "Changed"} type ${typeName()}`;
    return n.op === "delete" ? `Moved ${title(n)} to the trash` : `${DID[n.op]} ${title(n)}`;
  };
  return (
    <section class="ai-proposal ai-applied-card" aria-label={label()}>
      <p class="ai-proposal-head" role="status">
        <Show
          when={note()}
          fallback={
            <>
              {a().op === "type_create" ? "Created type " : "Changed type "}
              <a href={`/types/${encodeURIComponent(typeName())}`}>{typeName()}</a>
            </>
          }
        >
          {(n) => (
            <Show
              when={n().op !== "delete"}
              fallback={
                <>
                  Moved <strong>{title(n())}</strong> to the <a href="/trash">trash</a>, where you can restore it.
                </>
              }
            >
              {DID[n().op]} <a href={`/notes/${encodeURIComponent(n().note_id)}`}>{title(n())}</a>
            </Show>
          )}
        </Show>
      </p>
      <p class="muted">Made without asking, as Settings &gt; AI allows.</p>
    </section>
  );
}

/**
 * Apply all (ai D20): shown while a turn has two or more proposals pending. It applies them in
 * order and stops at the first that does not apply, whose card shows why.
 */
function ApplyAll(props: { turn: number; parts: Part[] }) {
  const [busy, setBusy] = createSignal(false);
  const count = () => props.parts.filter(pending).length;
  return (
    <Show when={count() >= 2 || busy()}>
      <div class="ai-proposal-actions ai-apply-all">
        <button
          type="button"
          class="primary"
          disabled={busy() || props.parts.some((p) => (p.kind === "proposal" || p.kind === "type_proposal" || p.kind === "download") && p.state === "applying")}
          onClick={async () => {
            setBusy(true);
            try {
              await applyAll(props.turn);
            } finally {
              setBusy(false);
            }
          }}
        >
          Apply all ({count()})
        </button>
      </div>
    </Show>
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

/** Where Settings > AI is: the home server's page in an app, this server's in the web app. */
function settingsUrl(): string {
  const server = inApp()?.server_url;
  return server ? `${server.replace(/\/+$/, "")}/settings/ai` : "/settings/ai";
}

/**
 * The provider and model picker: the providers Settings > AI has set up, from
 * `GET /api/ai/providers` in every client (ai D23), and each one's live model list (ai D6),
 * starting from the defaults chosen there.
 */
function Picker() {
  const [status, setStatus] = createSignal<AiProviders | null>(null);
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
      const s = await api.aiProviders();
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
        <Show
          when={inApp()}
          fallback={
            <p class="muted">
              No AI provider is set up. Add one in <a href="/settings/ai">Settings &gt; AI</a>.
            </p>
          }
        >
          <p class="muted">
            No AI provider is set up.{" "}
            <a href={settingsUrl()} target="_blank" rel="noopener">
              Set up AI in the web app
            </a>
            .
          </p>
        </Show>
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
          Ask about your notes. Answers come from {providerLabel(chat.provider)}; the notes it reads are sent there. Edits are proposals you apply, unless Settings &gt; AI lets them go ahead without asking.
        </p>
      </Show>
      <ol class="ai-turns" ref={list} aria-label="Conversation" aria-busy={chat.streaming}>
        <For each={chat.turns}>
          {(turn, t) => (
            <li class={`ai-turn ai-turn-${turn.role}`}>
              <Show when={turn.role === "user"} fallback={
                <>
                  <For each={turn.parts}>
                    {(part, i) => (
                      <Switch>
                        <Match when={part.kind === "text" && part}>{(p) => <Markdown text={p().text} />}</Match>
                        <Match when={part.kind === "tool" && part}>{(p) => <ToolStep part={p()} />}</Match>
                        <Match when={part.kind === "proposal" && part}>{(p) => <ProposalCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "type_proposal" && part}>{(p) => <TypeProposalCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "applied" && part}>{(p) => <AppliedCard applied={p().applied} />}</Match>
                        <Match when={part.kind === "download" && part}>{(p) => <DownloadCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "notice" && part}>{(p) => <Notice part={p()} />}</Match>
                      </Switch>
                    )}
                  </For>
                  <ApplyAll turn={t()} parts={turn.parts} />
                </>
              }>
                {/* What the user did with proposals since the last message, sent ahead of it (ai D29). */}
                <Show when={turn.outcomes?.length}>
                  <details class="ai-outcomes">
                    <summary>Applied changes</summary>
                    <ul>
                      <For each={turn.outcomes}>{(line) => <li>{line}</li>}</For>
                    </ul>
                  </details>
                </Show>
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
