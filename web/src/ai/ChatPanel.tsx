// The right sidebar's AI panel (ai D14, D23; agents G6, G18): pick an agent, Assistant by
// default, and one of its threads, ask about the vault, watch the answer stream, see which notes
// the model searched and read, and apply or discard the edits it proposes. Each conversation is a
// thread of the chosen agent, which its Chat tab lists too: the thread picker under the agent
// picker lists the agent's threads with the running and new-reply marks, starts a new one, and
// opens the shown one in the agent screen. A thread open here and in a Chat tab is one
// conversation, shown in both; each chooses its thread by itself. The panel remembers its agent
// and thread per browser. It shows in the web app and in the desktop and Android apps, whose chat
// runs on the home server. `Conversation` is the view the panel and the Chat tab share.

import { For, type JSX, Match, Show, Switch, createEffect, createRoot, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { agentLabel, agents } from "../agents";
import { type AgentThreadSummary, type AiModel, type AiProvider, type AiProviders, type Json, type NoteDoc, api, assetUrl } from "../api";
import { TypeIcon } from "../components/Icon";
import { formatValue } from "../components/Render";
import { Spinner } from "../components/SyncStatus";
import { activePane } from "../workspace/model";
import { navigatePane, workspace } from "../workspace/store";
import { AgentProposalCard } from "./AgentProposalCard";
import { blockLines, lineDiff } from "./diff";
import { DownloadCard } from "./DownloadCard";
import { Markdown } from "./markdown";
import { NewReplyMark, RunningMark } from "./RunningMark";
import { TaxonomyProposalCard } from "./TaxonomyProposalCard";
import { TypeProposalCard } from "./TypeProposalCard";
import { RUNNING_POLL_MS, hasNewReply, lastTurnEvent, turnEnded, turnsChanged, viewThread } from "./running";
import {
  type Applied,
  ChatContext,
  type ChatSession,
  type DraftRef,
  type NoteOp,
  type Part,
  type Proposal,
  type ProposalOp,
  chat,
  inApp,
  newConversation,
  openThread,
  panelSession,
  pending,
  providerLabel,
  setAgent,
  useChatSession,
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
    case "list_taxonomies":
      return "Listed the taxonomies";
    case "get_taxonomy":
      return `Read taxonomy ${s("name")}`;
    case "taxonomy_format":
      return "Read the taxonomy file format";
    case "propose_taxonomy_change":
      return `Checked a change to taxonomy ${s("name")}`;
    case "propose_taxonomy_delete":
      return `Checked a delete of taxonomy ${s("name")}`;
    case "list_agents":
      return "Listed the agents";
    case "get_agent":
      return `Read agent ${s("name")}`;
    case "propose_agent_change":
      return `Checked a change to agent ${s("name")}`;
    case "propose_agent_delete":
      return `Checked a delete of agent ${s("name")}`;
    case "draft_email":
      return "Drafted an email";
    case "draft_sms":
      return "Drafted an SMS";
    case "draft_event":
      return "Drafted a calendar invite";
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

/**
 * What a tool step is doing while it runs, for the activity line (ai D30): the step's words, as
 * `stepLabel` gives them once done, in the present.
 */
export function activeLabel(name: string, input: Json): string {
  const o = input && typeof input === "object" && !Array.isArray(input) ? input : {};
  switch (name) {
    case "search_notes":
      return "Searching notes";
    case "query_notes":
      return "Querying notes";
    case "get_note":
      return "Reading a note";
    case "backlinks":
      return "Looking up backlinks";
    case "read_asset":
      return "Reading a file";
    case "find_assets":
      return "Looking for files";
    case "propose_edit":
      return "Checking an edit";
    case "propose_create":
      return "Checking a new note";
    case "propose_archive":
      return o.archived === false ? "Checking an unarchive" : "Checking an archive";
    case "propose_delete":
      return "Checking a delete";
    case "list_types":
      return "Listing the types";
    case "get_type":
      return "Reading a type";
    case "type_format":
      return "Reading the type file format";
    case "propose_type_change":
      return "Checking a type change";
    case "list_taxonomies":
      return "Listing the taxonomies";
    case "get_taxonomy":
      return "Reading a taxonomy";
    case "taxonomy_format":
      return "Reading the taxonomy file format";
    case "propose_taxonomy_change":
      return "Checking a taxonomy change";
    case "propose_taxonomy_delete":
      return "Checking a taxonomy delete";
    case "list_agents":
      return "Listing the agents";
    case "get_agent":
      return "Reading an agent";
    case "propose_agent_change":
      return "Checking an agent change";
    case "propose_agent_delete":
      return "Checking an agent delete";
    case "draft_email":
      return "Drafting an email";
    case "draft_sms":
      return "Drafting an SMS";
    case "draft_event":
      return "Drafting a calendar invite";
    case "web_search":
      return "Searching the web";
    case "fetch_url":
      return "Reading a web page";
    case "download_asset":
      return "Checking a download";
    default:
      return `Running ${name}`;
  }
}

/** The type, taxonomy and agent tools (ai D18, D31, agents G20), whose steps return no notes. */
const TYPE_TOOLS = new Set([
  "list_types",
  "get_type",
  "type_format",
  "propose_type_change",
  "list_taxonomies",
  "get_taxonomy",
  "taxonomy_format",
  "propose_taxonomy_change",
  "propose_taxonomy_delete",
  "list_agents",
  "get_agent",
  "propose_agent_change",
  "propose_agent_delete",
]);

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

/** What a draft card says (outbox X9): what was drafted, for whom, and that it waits for review. */
export function draftText(d: DraftRef): string {
  const to = d.to.join(", ");
  const what =
    d.kind === "sms"
      ? `an SMS to ${to}`
      : d.kind === "event"
        ? `a calendar invite “${d.title}”${to ? ` for ${to}` : ""}`
        : `an email to ${to}`;
  return d.status === "approved" ? `Drafted ${what}, sending without review` : `Drafted ${what} for review`;
}

/** The compact card of a draft step (outbox X9): what it drafted, linking to the Outbox item. */
function DraftCard(props: { draft: DraftRef }) {
  return (
    <div class="ai-draft" role="group" aria-label="Outbox draft">
      <span>{draftText(props.draft)}</span>{" "}
      <a href={`/notes/${encodeURIComponent(props.draft.note_id)}`}>
        {props.draft.status === "approved" ? "Open the item" : "Review it in the Outbox"}
      </a>
    </div>
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
  const session = useChatSession();
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
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardProposal(props.turn, props.index)}>
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
type AppliedTaxonomy = Extract<Applied, { taxonomy_name: string }>;

/** What an applied taxonomy change did (ai D31). */
const TAXONOMY_DID: Record<AppliedTaxonomy["op"], string> = {
  taxonomy_change: "Changed taxonomy",
  taxonomy_create: "Created taxonomy",
  taxonomy_delete: "Deleted taxonomy",
};

type AppliedAgent = Extract<Applied, { agent_name: string }>;

/** What an applied agent change did (agents G20). */
const AGENT_DID: Record<AppliedAgent["op"], string> = {
  agent_change: "Changed agent",
  agent_create: "Created agent",
  agent_delete: "Deleted agent",
};

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
  const taxonomy = (): AppliedTaxonomy | undefined => {
    const x = a();
    return "taxonomy_name" in x ? x : undefined;
  };
  const agent = (): AppliedAgent | undefined => {
    const x = a();
    return "agent_name" in x ? x : undefined;
  };
  const title = (n: { title: Json; note_id: string }) => (typeof n.title === "string" && n.title ? n.title : n.note_id);
  const label = () => {
    const n = note();
    const x = taxonomy();
    const g = agent();
    if (g) return `${AGENT_DID[g.op]} ${g.agent_name}`;
    if (x) return `${TAXONOMY_DID[x.op]} ${x.taxonomy_name}`;
    if (!n) return `${a().op === "type_create" ? "Created" : "Changed"} type ${typeName()}`;
    return n.op === "delete" ? `Moved ${title(n)} to the trash` : `${DID[n.op]} ${title(n)}`;
  };
  return (
    <section class="ai-proposal ai-applied-card" aria-label={label()}>
      <p class="ai-proposal-head" role="status">
        <Show
          when={note()}
          fallback={
            <Show
              when={taxonomy()}
              fallback={
                <Show
                  when={agent()}
                  fallback={
                    <>
                      {a().op === "type_create" ? "Created type " : "Changed type "}
                      <a href={`/types/${encodeURIComponent(typeName())}`}>{typeName()}</a>
                    </>
                  }
                >
                  {(g) => (
                    <>
                      {AGENT_DID[g().op]}{" "}
                      <Show when={g().op !== "agent_delete"} fallback={<strong>{g().agent_name}</strong>}>
                        <a href={`/agents/${encodeURIComponent(g().agent_name)}`}>{g().agent_name}</a>
                      </Show>
                    </>
                  )}
                </Show>
              }
            >
              {(x) => (
                <>
                  {TAXONOMY_DID[x().op]}{" "}
                  <a href={x().op === "taxonomy_delete" ? "/settings/taxonomies" : `/settings/taxonomies/${encodeURIComponent(x().taxonomy_name)}`}>
                    {x().taxonomy_name}
                  </a>
                </>
              )}
            </Show>
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
      <p class="muted">Made without asking, as the agent's permissions allow.</p>
    </section>
  );
}

/**
 * Apply all (ai D20): shown while a turn has two or more proposals pending. It applies them in
 * order and stops at the first that does not apply, whose card shows why.
 */
function ApplyAll(props: { turn: number; parts: Part[] }) {
  const session = useChatSession();
  const [busy, setBusy] = createSignal(false);
  const count = () => props.parts.filter(pending).length;
  return (
    <Show when={count() >= 2 || busy()}>
      <div class="ai-proposal-actions ai-apply-all">
        <button
          type="button"
          class="primary"
          disabled={busy() || props.parts.some((p) => "state" in p && p.state === "applying")}
          onClick={async () => {
            setBusy(true);
            try {
              await session.applyAll(props.turn);
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

/**
 * The activity line above the message box while a turn runs (ai D30): a spinner, or a still dot
 * when motion is reduced, the current step's verb, and the seconds since the turn started. Only
 * the verb is in the polite status region, so a screen reader hears each step, not each second.
 */
function ActivityLine() {
  const { chat } = useChatSession();
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  const verb = () => {
    const a = chat.activity;
    if (!a || a.step === "thinking") return "Thinking";
    if (a.step === "writing") return "Writing";
    return activeLabel(a.name, a.input);
  };
  const seconds = () => Math.max(0, Math.floor((now() - (chat.startedAt ?? now())) / 1000));
  return (
    <div class="ai-activity">
      <Spinner />
      <span class="ai-activity-dot" aria-hidden="true" />
      <span role="status" aria-live="polite">
        {verb()}…
      </span>{" "}
      <span class="ai-activity-time">{seconds()}s</span>
    </div>
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
function Picker(props: { session: ChatSession }) {
  const { chat, setProvider, setModel } = props.session;
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

/**
 * The panel's agent picker (agents G6): Assistant first and chosen by default, then every enabled
 * agent. Choosing another starts a conversation with it.
 */
function AgentPicker() {
  const enabled = () => agents().filter((a) => a.enabled !== false && !a.error);
  return (
    <Show when={enabled().length > 0}>
      <select aria-label="Agent" class="ai-agent-picker" value={chat.agent} onChange={(e) => setAgent(e.currentTarget.value)}>
        <For each={enabled()}>{(a) => <option value={a.name}>{agentLabel(a)}</option>}</For>
      </select>
    </Show>
  );
}

/** The model an agent names itself, which the provider and model picker gives way to. */
function agentModel(name: string): string | null {
  const a = agents().find((x) => x.name === name);
  if (!a || (!a.provider && !a.model)) return null;
  return a.model ? a.model : `${providerLabel(a.provider)}'s default model`;
}

/** The nearest ancestor of `el` that scrolls, if any. */
function scrollParent(el: HTMLElement | undefined): HTMLElement | null {
  for (let at = el?.parentElement ?? null; at; at = at.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(at).overflowY)) return at;
  }
  return null;
}

/** How near the end, in pixels, still counts as at the end. */
const AT_END_PX = 48;

/**
 * A conversation as the panel and the Chat tab show it: its turns, with streaming text, tool
 * steps, proposal cards, Apply all, applied cards and notices, the activity line, and the box to
 * ask in. `noteId` is the note open beside it, if any. While it shows a thread, that thread has
 * no "New reply" (agents G17).
 */
export function Conversation(props: { session: ChatSession; noteId?: () => string; empty?: JSX.Element; placeholder?: string }) {
  const s = props.session;
  const chat = s.chat;
  const [draft, setDraft] = createSignal("");
  let list: HTMLOListElement | undefined;
  let form: HTMLFormElement | undefined;
  createEffect(
    on(
      () => chat.thread,
      (id) => {
        if (id) onCleanup(viewThread(id));
      },
    ),
  );
  // The message box sticks to the foot of the scrolling pane, and grows by the activity line
  // while a turn runs. At the end of the conversation, keep the end in view as the answer and the
  // box grow, so the last lines never slide under the box; scrolled up, leave the view alone.
  onMount(() => {
    const scroller = scrollParent(list);
    if (!scroller || !list || !form || typeof ResizeObserver === "undefined") return;
    const atEnd = () => scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= AT_END_PX;
    let pinned = atEnd();
    const onScroll = () => {
      pinned = atEnd();
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const grown = new ResizeObserver(() => {
      if (pinned) scroller.scrollTop = scroller.scrollHeight;
    });
    grown.observe(list);
    grown.observe(form);
    onCleanup(() => {
      grown.disconnect();
      scroller.removeEventListener("scroll", onScroll);
    });
  });
  function send() {
    const text = draft();
    if (!text.trim() || chat.streaming) return;
    setDraft("");
    void s.ask(text, props.noteId?.() || undefined);
    queueMicrotask(() => list?.lastElementChild?.scrollIntoView?.({ block: "end" }));
  }
  return (
    <ChatContext.Provider value={s}>
      <Show when={chat.turns.length === 0 && !chat.loading}>{props.empty}</Show>
      <Show when={chat.loading}>
        <p class="muted" role="status">
          Loading the thread…
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
                        <Match when={part.kind === "tool" && part}>
                          {(p) => (
                            <Show when={!p().isError && p().draft} fallback={<ToolStep part={p()} />}>
                              {(d) => <DraftCard draft={d()} />}
                            </Show>
                          )}
                        </Match>
                        <Match when={part.kind === "proposal" && part}>{(p) => <ProposalCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "type_proposal" && part}>{(p) => <TypeProposalCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "taxonomy_proposal" && part}>{(p) => <TaxonomyProposalCard part={p()} turn={t()} index={i()} />}</Match>
                        <Match when={part.kind === "agent_proposal" && part}>{(p) => <AgentProposalCard part={p()} turn={t()} index={i()} />}</Match>
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
        ref={form}
        class="ai-form"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        {/* Inside the form, which stays at the panel's foot, so the line sits right above the box. */}
        <Show when={chat.streaming && chat.activity}>
          <ActivityLine />
        </Show>
        <textarea
          aria-label="Ask AI"
          placeholder={props.placeholder ?? "Ask about your notes"}
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
          <button type="button" onClick={s.stop}>
            Stop
          </button>
        </Show>
      </form>
    </ChatContext.Provider>
  );
}

/** The provider and model picker, or the model the conversation's agent names itself. */
export function ModelPicker(props: { session: ChatSession }) {
  return (
    <Show when={agentModel(props.session.chat.agent)} fallback={<Picker session={props.session} />}>
      {(m) => <p class="muted ai-agent-model">Model: {m()}</p>}
    </Show>
  );
}

/** Where the panel keeps its choice of agent and thread, per browser (agents G18). */
export const PANEL_KEY = "thevault.ai.panel";

interface PanelChoice {
  agent: string;
  thread: string | null;
}

function savedChoice(): PanelChoice | null {
  try {
    const raw = JSON.parse(localStorage.getItem(PANEL_KEY) ?? "null") as Partial<PanelChoice> | null;
    if (!raw || typeof raw.agent !== "string") return null;
    return { agent: raw.agent, thread: typeof raw.thread === "string" ? raw.thread : null };
  } catch {
    return null;
  }
}

/** Whether the panel took its saved choice yet; until then nothing is saved over it. */
let restored = false;

/** Test-only: take the saved choice again, as after a reload. */
export function forgetPanelChoice() {
  restored = false;
}

createRoot(() =>
  createEffect(() => {
    const choice: PanelChoice = { agent: chat.agent, thread: chat.thread };
    if (!restored) return;
    try {
      localStorage.setItem(PANEL_KEY, JSON.stringify(choice));
    } catch {
      // Private browsing: the choice lasts until the page closes.
    }
  }),
);

/**
 * Open the panel on the agent and thread it showed last, once the agent list is known, unless a
 * conversation started first. A thread archived or deleted since gives way to a new one.
 */
async function restoreChoice() {
  const saved = savedChoice();
  if (!saved || chat.thread || chat.turns.length > 0 || chat.streaming) return;
  if (!agents().some((a) => a.name === saved.agent && a.enabled !== false && !a.error)) return;
  setAgent(saved.agent);
  if (!saved.thread) return;
  const got = await openThread(saved.thread);
  if ((got === "archived" || got === "gone") && chat.thread === saved.thread) newConversation();
}

/** Where thread `id` of agent `agent` opens in the agent screen's Chat tab. */
export function threadHref(agent: string, id: string): string {
  return `/agents/${encodeURIComponent(agent)}/chat?thread=${encodeURIComponent(id)}`;
}

/**
 * The panel's thread picker (agents G18), under the agent picker: the chosen agent's threads,
 * newest activity first and archived left out, each with the running spinner or the "New reply"
 * dot (agents G17); New thread at the top; and Open in agent screen. It reads the threads again
 * when it opens, when a turn of the agent starts or ends, when the panel's thread changes, and
 * every few seconds while one runs, as the apps need.
 */
function ThreadPicker() {
  const [threads, setThreads] = createSignal<AgentThreadSummary[]>([]);
  const [open, setOpen] = createSignal(false);
  const listed = () => agents().some((a) => a.name === chat.agent);
  let anchor: HTMLButtonElement | undefined;
  let asked = 0;
  let poll: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(poll));
  async function refresh() {
    const mine = ++asked;
    clearTimeout(poll);
    if (!untrack(listed)) {
      setThreads([]);
      return;
    }
    const agent = untrack(() => chat.agent);
    try {
      const got = await api.agentThreads(agent);
      if (mine !== asked) return;
      // A thread that stopped running since the last read answered: a new reply, unless open.
      const was = new Set(threads().filter((t) => t.running).map((t) => t.id));
      for (const t of got.threads) if (was.has(t.id) && !t.running) turnEnded(t.id);
      setThreads(got.threads);
      if (got.threads.some((t) => t.running)) poll = setTimeout(() => void refresh(), RUNNING_POLL_MS);
    } catch {
      if (mine === asked) setThreads([]);
    }
  }
  createEffect(
    on([() => chat.agent, listed], () => {
      setThreads([]);
      setOpen(false);
      void refresh();
    }),
  );
  createEffect(on(() => chat.thread, () => void refresh(), { defer: true }));
  createEffect(
    on(
      turnsChanged,
      () => {
        const e = lastTurnEvent();
        if (!e || e.agent === chat.agent) void refresh();
      },
      { defer: true },
    ),
  );
  const titleOf = (t: AgentThreadSummary) => t.title || "Untitled thread";
  const current = () => {
    const id = chat.thread;
    if (!id) return "New thread";
    const t = threads().find((x) => x.id === id);
    return t ? titleOf(t) : "Untitled thread";
  };
  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) anchor?.focus();
  }
  function choose(id: string | null) {
    close(true);
    if (id === null) newConversation();
    else if (chat.thread !== id) void openThread(id);
  }
  function inScreen() {
    const id = chat.thread;
    if (!id) return;
    navigatePane(activePane(workspace()), threadHref(chat.agent, id));
  }

  function Menu() {
    let el!: HTMLDivElement;
    const items = () => [...el.querySelectorAll<HTMLElement>("[role^=menuitem]")];
    onMount(() => {
      (items().find((i) => i.getAttribute("aria-checked") === "true") ?? items()[0])?.focus();
      const outside = (e: PointerEvent) => {
        if (!el.contains(e.target as Node) && !anchor?.contains(e.target as Node)) close(false);
      };
      document.addEventListener("pointerdown", outside, true);
      // Escape closes the menu wherever focus is: a refresh of the thread list can leave focus
      // outside it.
      const escape = (e: KeyboardEvent) => {
        if (e.key === "Escape" && !el.contains(document.activeElement)) {
          e.preventDefault();
          close(true);
        }
      };
      document.addEventListener("keydown", escape);
      onCleanup(() => {
        document.removeEventListener("pointerdown", outside, true);
        document.removeEventListener("keydown", escape);
      });
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
      <div ref={el} class="pin-menu ai-thread-menu" role="menu" aria-label="Threads" onKeyDown={onKey}>
        <button type="button" role="menuitem" onClick={() => choose(null)}>
          <TypeIcon name="plus" size={16} />
          <span>New thread</span>
        </button>
        <For each={threads()}>
          {(t) => (
            <button type="button" role="menuitemradio" aria-checked={chat.thread === t.id} onClick={() => choose(t.id)}>
              <span class="ai-thread-title">{titleOf(t)}</span>
              <Show when={t.running} fallback={<Show when={hasNewReply(t.id) && chat.thread !== t.id}><NewReplyMark /></Show>}>
                <RunningMark />
              </Show>
            </button>
          )}
        </For>
      </div>
    );
  }

  return (
    <Show when={listed()}>
      <div class="ai-threads">
        <div class="ai-thread-picker">
          <button
            ref={anchor}
            type="button"
            class="ai-thread-button"
            aria-haspopup="menu"
            aria-expanded={open()}
            aria-label={`Thread: ${current()}`}
            title={current()}
            onClick={() => {
              if (!open()) void refresh();
              setOpen(!open());
            }}
          >
            <span class="ai-thread-current">{current()}</span>
            <Show when={chat.thread && chat.streaming}>
              <RunningMark />
            </Show>
            <TypeIcon name="chevron-down" size={14} />
          </button>
          <Show when={open()}>
            <Menu />
          </Show>
        </div>
        <button type="button" class="link" onClick={inScreen} disabled={!chat.thread}>
          Open in agent screen
        </button>
      </div>
    </Show>
  );
}

/** The AI panel. `noteId` is the note open in the active pane, if any. */
export function ChatPanel(props: { noteId: () => string }) {
  const session = panelSession;
  const listed = () => agents().some((a) => a.name === chat.agent);
  // The agent and thread shown last, once the agents are known.
  createEffect(() => {
    if (restored || agents().length === 0) return;
    restored = true;
    untrack(() => void restoreChoice());
  });

  return (
    <div class="ai-panel">
      <div class="ai-toolbar">
        <AgentPicker />
        <ModelPicker session={session} />
        <Show when={!listed()}>
          <button type="button" class="link" onClick={newConversation} disabled={chat.turns.length === 0 && !chat.streaming}>
            New conversation
          </button>
        </Show>
      </div>
      <ThreadPicker />
      <Conversation
        session={session}
        noteId={props.noteId}
        empty={
          <p class="muted ai-empty">
            Ask about your notes. Answers come from {providerLabel(chat.provider)}; the notes it reads are sent there. Edits are proposals you apply, unless the agent's permissions let them go ahead without asking.
          </p>
        }
      />
    </div>
  );
}
