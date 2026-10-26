// The right sidebar's AI panel (ai D14, D23; agents G6, G18): pick an agent, Assistant by
// default, and one of its threads, ask about the vault, watch the answer stream, see which notes
// the model searched and read, and apply or discard the edits it proposes. Each conversation is a
// thread of the chosen agent, which the agent screen's session picker lists too: the thread
// picker under the agent picker lists the agent's threads with the running and new-reply marks,
// starts a new one, and opens the shown one in the agent screen. A thread open here and in the
// agent screen is one conversation, shown in both; each chooses its thread by itself. The panel
// remembers its agent and thread per browser. It shows in the web app and in the desktop and
// Android apps, whose chat runs on the home server. `Conversation` is the view the panel and the
// agent screen share, with the provider and model picker at the foot of its message box.

import {
  For,
  type JSX,
  Match,
  Show,
  Switch,
  createContext,
  createEffect,
  createMemo,
  createRoot,
  createSignal,
  on,
  onCleanup,
  onMount,
  untrack,
  useContext,
} from "solid-js";
import { agentLabel, agents } from "../agents";
import { type AgentThreadSummary, type AiAccountChoice, type AiAccounts, type AiModel, type Json, type NoteDoc, api, assetUrl, isSubSession } from "../api";
import { TypeIcon } from "../components/Icon";
import { formatValue } from "../components/Render";
import { Spinner } from "../components/SyncStatus";
import { activePane } from "../workspace/model";
import { navigatePane, openBeside, workspace } from "../workspace/store";
import { paneAt, wantsNewTab } from "../workspace/events";
import { AgentProposalCard } from "./AgentProposalCard";
import { MemoryProposalCard, memoryHref } from "./MemoryProposalCard";
import { accountChoiceLabel, accountFor, usedPercent } from "./accounts";
import { blockLines, lineDiff } from "./diff";
import { DownloadCard } from "./DownloadCard";
import { Markdown } from "./markdown";
import { NewReplyMark, RunningMark } from "./RunningMark";
import { TaxonomyProposalCard } from "./TaxonomyProposalCard";
import { TypeProposalCard } from "./TypeProposalCard";
import { outboxItemHref } from "../outbox/model";
import { SkillProposalCard, skillHref } from "./SkillProposalCard";
import { ToolProposalCard, toolHref } from "./ToolProposalCard";
import { RUNNING_POLL_MS, hasNewReply, lastTurnEvent, turnEnded, turnsChanged, viewThread } from "./running";
import {
  type Applied,
  ChatContext,
  type Attachment,
  type ChatSession,
  type ChatState,
  type ContextUse,
  attachmentsSynced,
  type DraftRef,
  type NoteOp,
  type BrowserRef,
  type ScriptStep,
  type Part,
  type Proposal,
  type ProposalOp,
  type ProposalState,
  chat,
  inApp,
  newConversation,
  openThread,
  ownsModel,
  panelSession,
  pending,
  setAgent,
  skillWhat,
  subagentOf,
  toolWhat,
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
    case "propose_type_delete":
      return `Checked deleting type ${s("name")}`;
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
    case "list_agent_memories":
      return `Listed the memories of agent ${s("agent")}`;
    case "get_agent_memory":
      return "Read another agent's memory";
    case "propose_agent_memory_change":
      return o.id ? `Checked a change to a memory of agent ${s("agent")}` : `Checked a new memory of agent ${s("agent")}`;
    case "propose_agent_memory_delete":
      return "Checked forgetting another agent's memory";
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
    case "browser_run":
      return `Ran ${oneLine(s("command"))} in the browser box`;
    case "browser_file":
      return `Read ${s("path")} from the browser box`;
    case "browser_save":
      return `Saved ${s("path")} from the browser box`;
    case "browser_capture":
      return s("what") === "image" ? "Saved an image from the page" : "Saved a screenshot of the page";
    case "use_skill":
      return `Used skill ${s("name")}`;
    case "read_skill_file":
      return `Read ${s("path")} from skill ${s("name")}`;
    case "install_skill_in_browser":
      return `Copied skill ${s("name")} into the browser box`;
    case "list_skills":
      return "Listed the skills";
    case "get_skill":
      return `Read skill ${s("name")}`;
    case "propose_skill_change":
      return `Checked SKILL.md of skill ${s("name")}`;
    case "propose_skill_file_change":
      return `Checked ${s("path")} of skill ${s("name")}`;
    case "propose_skill_file_delete":
      return `Checked a delete of ${s("path")} from skill ${s("name")}`;
    case "propose_skill_delete":
      return `Checked a delete of skill ${s("name")}`;
    case "run_script":
      return "Ran wscript code";
    case "list_tools":
      return "Listed the script tools";
    case "get_tool":
      return `Read tool ${s("name")}`;
    case "propose_tool_change":
      return `Checked tool ${s("name")}`;
    case "propose_tool_delete":
      return `Checked a delete of tool ${s("name")}`;
    case "start_subagent":
      return s("agent") ? `Delegated to ${s("agent")}` : "Started a sub agent";
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
    case "propose_type_delete":
      return "Checking a type delete";
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
    case "list_agent_memories":
      return "Listing another agent's memories";
    case "get_agent_memory":
      return "Reading another agent's memory";
    case "propose_agent_memory_change":
      return "Checking a memory change";
    case "propose_agent_memory_delete":
      return "Checking forgetting a memory";
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
    case "browser_run":
      return "Running a command in the browser box";
    case "browser_file":
      return "Reading a file from the browser box";
    case "browser_save":
      return "Checking a file from the browser box";
    case "browser_capture":
      return "Capturing from the page";
    case "use_skill":
      return "Reading a skill";
    case "read_skill_file":
      return "Reading a skill's file";
    case "install_skill_in_browser":
      return "Copying a skill into the browser box";
    case "list_skills":
      return "Listing the skills";
    case "get_skill":
      return "Reading a skill";
    case "propose_skill_change":
    case "propose_skill_file_change":
    case "propose_skill_file_delete":
    case "propose_skill_delete":
      return "Checking a skill change";
    case "run_script":
      return "Running wscript code";
    case "list_tools":
      return "Listing the script tools";
    case "get_tool":
      return "Reading a script tool";
    case "propose_tool_change":
    case "propose_tool_delete":
      return "Checking a tool change";
    case "start_subagent":
      return "Waiting for sub agents";
    default:
      return `Running ${name}`;
  }
}

/** A command's first line, at most 80 characters, for a step's words. */
export function oneLine(command: string): string {
  const first = command.trim().split("\n")[0] ?? "";
  const more = first.length > 80 || command.trim().includes("\n");
  return `${first.slice(0, 80)}${more ? "…" : ""}`;
}

/** What a browser step says once done (browser B5). */
function browserStatus(b: BrowserRef): string {
  if (b.kind === "run") return b.timed_out ? "timed out" : `exit ${b.exit ?? "?"}`;
  if (b.kind === "save") return b.asset_id ? `stored as ${b.filename ?? "an asset"}` : "proposed";
  return b.media_type || "done";
}

/** The type, taxonomy, agent and skill tools (ai D18, D31, agents G20, skills S6), whose steps return no notes. */
const TYPE_TOOLS = new Set([
  "list_types",
  "get_type",
  "type_format",
  "propose_type_change",
  "propose_type_delete",
  "list_taxonomies",
  "get_taxonomy",
  "taxonomy_format",
  "propose_taxonomy_change",
  "propose_taxonomy_delete",
  "list_agents",
  "get_agent",
  "propose_agent_change",
  "propose_agent_delete",
  // The skill tools (skills S6) return no notes either.
  "use_skill",
  "read_skill_file",
  "install_skill_in_browser",
  // And the skill management tools (skills S9).
  "list_skills",
  "get_skill",
  "propose_skill_change",
  "propose_skill_file_change",
  "propose_skill_file_delete",
  "propose_skill_delete",
  // And the script tool tools (script-tools T3).
  "list_tools",
  "get_tool",
  "propose_tool_change",
  "propose_tool_delete",
]);

/** What a script step says once done (script-tools T6): its outcome and how long it took. */
function scriptStatus(s: ScriptStep): string {
  const took = s.ms !== undefined ? ` in ${s.ms} ms` : "";
  return s.outcome === "ok" ? `ok${took}` : `failed${s.error_kind ? ` (${s.error_kind})` : ""}${took}`;
}

/** The host of `url`, or `url` itself when it is not a URL. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** Whether a tool drives the browser box (browser B5). */
const isBrowserTool = (name: string) =>
  name === "browser_run" ||
  name === "browser_file" ||
  name === "browser_save" ||
  name === "browser_capture" ||
  name === "install_skill_in_browser";

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

/** The code a run_script step ran, from its input. */
function codeOf(input: Json): string {
  const o = input && typeof input === "object" && !Array.isArray(input) ? input : {};
  return typeof o.code === "string" ? o.code : "";
}

function ToolStep(props: { part: Extract<Part, { kind: "tool" }> }) {
  const status = () => {
    if (!props.part.done) return "running";
    if (props.part.isError) return "failed";
    if (props.part.script) return scriptStatus(props.part.script);
    if (props.part.browser) return browserStatus(props.part.browser);
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
        {/* Watch what the agent does: the browser box's desktop in a pane beside this conversation
            (browser B4); a Ctrl or middle click opens it in a background tab instead. The web app
            only, as the apps cannot open the desktop. */}
        <Show when={isBrowserTool(props.part.name) && !inApp()}>
          <a
            class="ai-step-watch"
            href="/browser-box"
            title="Show the browser box's desktop beside this conversation"
            onClick={(e) => {
              if (wantsNewTab(e)) return;
              e.preventDefault();
              e.stopPropagation();
              openBeside(paneAt(e.currentTarget), "/browser-box");
            }}
          >
            Watch
          </a>
        </Show>
        <Show when={image()}>
          {(a) => (
            <a class="ai-step-thumb" href={assetUrl(a().id)} target="_blank" rel="noopener">
              <img src={assetUrl(a().id)} alt={a().filename} loading="lazy" onError={() => setBroken(true)} />
            </a>
          )}
        </Show>
        {/* A browser_file image is not in the vault: the step's thumbnail comes in its summary (browser B5). */}
        <Show when={props.part.browser?.thumbnail}>
          {(src) => (
            <span class="ai-step-thumb">
              <img src={src()} alt={props.part.browser?.path ?? "screenshot"} />
            </span>
          )}
        </Show>
      </summary>
      <Show when={props.part.browser?.kind === "run" && props.part.browser}>
        {(b) => (
          <div class="ai-step-browser">
            <p class="muted">
              <code>{b().command}</code>
              {b().ms !== undefined ? ` (${b().ms} ms)` : ""}
            </p>
            <Show when={b().stdout}>
              <pre class="ai-step-output" aria-label="Output">
                {b().stdout}
              </pre>
            </Show>
            <Show when={b().stderr}>
              <pre class="ai-step-output ai-step-stderr" aria-label="Errors">
                {b().stderr}
              </pre>
            </Show>
          </div>
        )}
      </Show>
      <Show when={props.part.name === "run_script" ? codeOf(props.part.input) : ""}>
        {(code) => (
          <pre class="ai-step-output" aria-label="Code">
            {code()}
          </pre>
        )}
      </Show>
      <Show when={props.part.script}>
        {(s) => (
          <div class="ai-step-script">
            <p class="muted">
              {s().instructions !== undefined ? `${s().instructions} instructions` : ""}
              <Show when={s().ref?.hash}>
                {(h) => <span title={h()}>{` · version ${h().slice(0, 12)}`}</span>}
              </Show>
            </p>
            <Show when={s().log}>
              <pre class="ai-step-output" aria-label="Log">
                {s().log}
              </pre>
            </Show>
          </div>
        )}
      </Show>
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

/** What a draft card says (outbox X9, X14): what was drafted, for whom, and that it waits in Draft. */
export function draftText(d: DraftRef): string {
  const to = d.to.join(", ");
  const what =
    d.kind === "sms"
      ? `an SMS to ${to}`
      : d.kind === "event"
        ? `a calendar invite “${d.title}”${to ? ` for ${to}` : ""}`
        : `an email to ${to}`;
  return d.status === "ready" ? `Drafted ${what}, sending without review` : `Drafted ${what}`;
}

/** The compact card of a draft step (outbox X9): what it drafted, linking to the Outbox item. */
function DraftCard(props: { draft: DraftRef }) {
  return (
    <div class="ai-draft" role="group" aria-label="Outbox draft">
      <span>{draftText(props.draft)}</span>{" "}
      <a href={outboxItemHref(props.draft.note_id)}>
        {props.draft.status === "ready" ? "Open the item" : "Open it in the Outbox"}
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

type TypeOp = "type_change" | "type_create" | "type_delete";

/** What an applied type change did, for its card. */
const TYPE_DID: Record<TypeOp, string> = {
  type_change: "Changed",
  type_create: "Created",
  type_delete: "Deleted",
};

/** What an applied taxonomy change did (ai D31). */
const TAXONOMY_DID: Record<AppliedTaxonomy["op"], string> = {
  taxonomy_change: "Changed taxonomy",
  taxonomy_create: "Created taxonomy",
  taxonomy_delete: "Deleted taxonomy",
};

type AppliedAgent = Extract<Applied, { agent_name: string }>;
type AppliedMemory = Extract<Applied, { memory_agent: string }>;

/** What an applied change to another agent's memories did (agents G21). */
const MEMORY_DID: Record<AppliedMemory["op"], string> = {
  agent_memory_create: "Added a memory of",
  agent_memory_change: "Changed a memory of",
  agent_memory_delete: "Moved to the trash a memory of",
};

/** A change to another agent's memories the AI made itself (agents G21). */
function MemoryAppliedCard(props: { applied: AppliedMemory }) {
  const a = () => props.applied;
  return (
    <section class="ai-proposal ai-applied-card" aria-label={`${MEMORY_DID[a().op]} ${a().memory_agent}`}>
      <p class="ai-proposal-head" role="status">
        {MEMORY_DID[a().op]} <a href={memoryHref(a().memory_agent)}>{a().memory_agent}</a>
      </p>
      <p class="muted">Made without asking, as the agent's permissions allow.</p>
    </section>
  );
}

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
  const memory = (): AppliedMemory | undefined => {
    const x = props.applied;
    return "memory_agent" in x ? x : undefined;
  };
  const skill = (): AppliedSkill | undefined => {
    const x = props.applied;
    return "skill_name" in x ? x : undefined;
  };
  const tool = (): AppliedTool | undefined => {
    const x = props.applied;
    return "tool_name" in x ? x : undefined;
  };
  return (
    <Show when={tool()} fallback={
    <Show when={skill()} fallback={
      <Show when={memory()} fallback={<ChangeAppliedCard applied={props.applied} />}>
        {(m) => <MemoryAppliedCard applied={m()} />}
      </Show>
    }>
      {(s) => <SkillAppliedCard applied={s()} />}
    </Show>
    }>
      {(x) => <ToolAppliedCard applied={x()} />}
    </Show>
  );
}

type AppliedTool = Extract<Applied, { tool_name: string }>;

const TOOL_DID: Record<AppliedTool["op"], string> = {
  tool_create: "Created tool",
  tool_change: "Changed tool",
  tool_delete: "Deleted tool",
};

/** A script tool change the AI made itself (script-tools T3), linking to the tool. */
function ToolAppliedCard(props: { applied: AppliedTool }) {
  const a = () => props.applied;
  return (
    <section class="ai-proposal ai-applied-card" aria-label={`${TOOL_DID[a().op]} ${a().tool_name}`}>
      <p class="ai-proposal-head" role="status">
        {TOOL_DID[a().op]}{" "}
        <Show when={a().op !== "tool_delete"} fallback={<strong>{a().tool_name}</strong>}>
          <a href={toolHref(a().tool_name)}>{a().tool_name}</a>
        </Show>
      </p>
      <p class="muted">Made without asking, as the agent's permissions allow.</p>
    </section>
  );
}

type AppliedSkill = Extract<Applied, { skill_name: string }>;

/** What an applied skill change did (skills S9), before the skill's name. */
function skillDid(a: AppliedSkill): string {
  switch (a.op) {
    case "skill_create":
      return "Created skill";
    case "skill_change":
      return "Changed SKILL.md of skill";
    case "skill_file":
      return `Wrote ${a.path ?? ""} in skill`;
    case "skill_file_delete":
      return `Deleted ${a.path ?? ""} from skill`;
    default:
      return "Deleted skill";
  }
}

/** A skill change the AI made itself (skills S9), linking to the skill. */
function SkillAppliedCard(props: { applied: AppliedSkill }) {
  const a = () => props.applied;
  return (
    <section class="ai-proposal ai-applied-card" aria-label={`${skillDid(a())} ${a().skill_name}`}>
      <p class="ai-proposal-head" role="status">
        {skillDid(a())}{" "}
        <Show when={a().op !== "skill_delete"} fallback={<strong>{a().skill_name}</strong>}>
          <a href={skillHref(a().skill_name)}>{a().skill_name}</a>
        </Show>
      </p>
      <p class="muted">Made without asking, as the agent's permissions allow.</p>
    </section>
  );
}

/** An applied change other than to a memory. */
function ChangeAppliedCard(props: { applied: Applied }) {
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
    if (!n) return `${TYPE_DID[a().op as TypeOp]} type ${typeName()}`;
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
                      {`${TYPE_DID[a().op as TypeOp]} type `}
                      <Show when={a().op !== "type_delete"} fallback={<strong>{typeName()}</strong>}>
                        <a href={`/types/${encodeURIComponent(typeName())}`}>{typeName()}</a>
                      </Show>
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

/** A token count as people read it: 950, 12.4k, 1.2M. */
export function tokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${n < 10_000 ? (n / 1000).toFixed(1) : Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}

/** The share of the window, in percent, past which a thread compacts itself, as Settings > AI has it. */
const [compactAt, setCompactAt] = createSignal(80);
let compactAtAsked = false;

/**
 * How full the context was after the last call, above the message box: tokens used over the
 * window, as a bar that turns amber past the compaction point and red near the window.
 */
function ContextMeter(props: { use: ContextUse }) {
  if (!compactAtAsked) {
    compactAtAsked = true;
    void loadAccounts().catch(() => undefined);
  }
  const pct = () => Math.min(100, Math.round((props.use.used / Math.max(1, props.use.window)) * 100));
  const level = () => (pct() >= 95 ? "full" : pct() >= compactAt() ? "high" : "ok");
  return (
    <div
      class={`ai-context-meter ai-context-${level()}`}
      data-testid="context-meter"
      title={`The last request held ${props.use.used.toLocaleString()} of ${props.use.window.toLocaleString()} tokens. Type /compact to summarize the conversation.`}
    >
      <span class="ai-context-bar" role="meter" aria-label="Context used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct()}>
        <span style={{ width: `${pct()}%` }} />
      </span>
      <span class="ai-context-text">
        {tokens(props.use.used)} / {tokens(props.use.window)}
      </span>
    </div>
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
    if (a.step === "compacting") return "Compacting the conversation";
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

/**
 * The accounts Settings > AI has, from `GET /api/ai/accounts` in every client (accounts A7), as the
 * last picker read them: each picker reads them afresh when it opens.
 */
const [accountList, setAccountList] = createSignal<AiAccounts | null>(null);

async function loadAccounts(): Promise<AiAccounts> {
  const got = await api.aiAccounts();
  setAccountList(got);
  if (got.context?.compact_at) setCompactAt(got.context.compact_at);
  return got;
}

/** Where Settings > AI is: the home server's page in an app, this server's in the web app. */
function settingsUrl(): string {
  const server = inApp()?.server_url;
  return server ? `${server.replace(/\/+$/, "")}/settings/ai` : "/settings/ai";
}

/** What the picker says when Settings > AI has no account set up. */
function NoAccount() {
  return (
    <Show
      when={inApp()}
      fallback={
        <p class="muted">
          No AI account is set up. Add one in <a href="/settings/ai">Settings &gt; AI</a>.
        </p>
      }
    >
      <p class="muted">
        No AI account is set up.{" "}
        <a href={settingsUrl()} target="_blank" rel="noopener">
          Set up AI in the web app
        </a>
        .
      </p>
    </Show>
  );
}

/** How much of the chosen account's plan is used, beside the picker, when it has a reading (accounts A13). */
function UsedBadge(props: { account: AiAccountChoice | null | undefined }) {
  const used = () => usedPercent(props.account?.usage);
  return (
    <Show when={used() !== null}>
      <span
        class="ai-used"
        data-testid="account-used"
        title={props.account!.usage!.windows.map((w) => `${w.label}: ${Math.round(w.used_percent)}% used`).join("\n")}
      >
        {used()}% used
      </span>
    </Show>
  );
}

/**
 * The account and model picker: the accounts Settings > AI has set up (accounts A7), and each
 * one's live model list (ai D6), starting from the default account and its default model.
 */
function Picker(props: { session: ChatSession }) {
  const { chat, setAccount, setModel } = props.session;
  const [models, setModels] = createSignal<AiModel[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const configured = () => (accountList()?.accounts ?? []).filter((a) => a.configured);
  const byId = (id: string | null) => accountList()?.accounts.find((a) => a.id === id);
  // A picker gone, its agent changed to one that names its own model, chooses nothing more.
  let gone = false;
  onCleanup(() => (gone = true));
  async function loadModels(id: string) {
    if (gone) return;
    setAccount(id, chat.account === id ? chat.model : (byId(id)?.model ?? null));
    try {
      const got = await api.aiModels(id);
      if (gone || chat.account !== id) return;
      setModels(got.models);
      setError(null);
      if (!chat.model || !got.models.some((m) => m.id === chat.model)) setModel(got.model);
    } catch (e) {
      if (chat.account !== id) return;
      setModels([]);
      setError(e instanceof Error ? e.message : String(e));
    }
  }
  onMount(async () => {
    try {
      const s = await loadAccounts();
      setLoaded(true);
      const ready = configured().map((a) => a.id);
      const pick = [chat.account, s.default].find((id): id is string => !!id && ready.includes(id)) ?? ready[0];
      if (pick) await loadModels(pick);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  });
  return (
    <div class="ai-picker">
      <Show when={loaded() && configured().length === 0}>
        <NoAccount />
      </Show>
      <Show when={loaded() && configured().length > 0}>
        <select aria-label="Account" value={chat.account ?? ""} onChange={(e) => void loadModels(e.currentTarget.value)}>
          <For each={accountList()?.accounts ?? []}>
            {(a) => (
              <option value={a.id} disabled={!a.configured}>
                {accountChoiceLabel(a)}
              </option>
            )}
          </For>
        </select>
        <UsedBadge account={byId(chat.account)} />
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

/**
 * The account and model picker of an agent that names its own: the agent's choice first, which
 * sends none, so the server runs the agent's own; another picked overrides it for the
 * conversation's next turns (agents G1). Nothing is chosen for the user, so another agent's or the
 * Assistant's choice never carries over.
 */
function OwnModelPicker(props: { session: ChatSession; agent: string }) {
  const { chat, setAccount, setModel } = props.session;
  const [list, setList] = createSignal<{ account: string; models: AiModel[]; model: string | null } | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const configured = () => (accountList()?.accounts ?? []).filter((a) => a.configured);
  const agent = () => agents().find((a) => a.name === props.agent);
  /** The account the agent runs on: its own, else Settings > AI's default (accounts A5). */
  const own = () => accountFor(accountList(), agent()?.account);
  /** The account whose models are listed: the one picked, or the agent's. */
  const shown = () => chat.account ?? own()?.id ?? null;
  const byId = (id: string | null) => accountList()?.accounts.find((a) => a.id === id);
  const models = () => (list()?.account === shown() ? list()!.models : []);
  const defaultOf = (id: string | null) => (list()?.account === id ? list()!.model : null) ?? byId(id)?.model ?? null;
  const ownModel = () => agent()?.model ?? defaultOf(own()?.id ?? null);
  /** The agent's own model is its first option, so the list leaves it out. */
  const listed = () => models().filter((m) => chat.account || m.id !== agent()?.model);
  onMount(async () => {
    try {
      await loadAccounts();
      setLoaded(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  });
  createEffect(
    on([shown, accountList], async ([id]) => {
      if (!id || !byId(id)?.configured) return;
      try {
        const got = await api.aiModels(id);
        if (shown() !== id) return;
        setList({ account: id, models: got.models, model: got.model });
        setError(null);
      } catch (e) {
        if (shown() !== id) return;
        setList(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    }),
  );
  return (
    <div class="ai-picker">
      <Show when={loaded() && configured().length === 0}>
        <NoAccount />
      </Show>
      <Show when={loaded() && configured().length > 0}>
        <select aria-label="Account" value={chat.account ?? ""} onChange={(e) => setAccount(e.currentTarget.value || null, null)}>
          <option value="">Agent's account ({own() ? accountChoiceLabel(own()!) : "Settings > AI"})</option>
          <For each={(accountList()?.accounts ?? []).filter((a) => a.id !== own()?.id)}>
            {(a) => (
              <option value={a.id} disabled={!a.configured}>
                {accountChoiceLabel(a)}
              </option>
            )}
          </For>
        </select>
        <UsedBadge account={byId(shown())} />
        <select aria-label="Model" value={chat.model ?? ""} onChange={(e) => setModel(e.currentTarget.value || null)}>
          <Show when={chat.account} fallback={<option value="">Agent's model ({ownModel() ?? "account default"})</option>}>
            <option value="">Account default{defaultOf(shown()) ? ` (${defaultOf(shown())})` : ""}</option>
          </Show>
          <Show when={chat.model && !listed().some((m) => m.id === chat.model)}>
            <option value={chat.model!}>{chat.model}</option>
          </Show>
          <For each={listed()}>{(m) => <option value={m.id}>{m.display_name || m.id}</option>}</For>
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

/** The name of the account the conversation's next turn goes to, as the disclosure line names it. */
function accountShown(chat: ChatState): string {
  const list = accountList();
  const picked = list?.accounts.find((a) => a.id === chat.account);
  const agent = agents().find((a) => a.name === chat.agent);
  return (picked ?? accountFor(list, agent?.account))?.name ?? "your AI account";
}

/** A file in the box: uploading, stored as an asset, or failed. */
interface Upload {
  key: number;
  filename: string;
  size: number;
  state: "uploading" | "ready" | "failed";
  asset?: Attachment;
  error?: string;
}

let uploads = 0;

/** A pasted image arrives named `image.png`: give it a name of its own. */
function pastedName(file: File): File {
  if (!/^image\.\w+$/.test(file.name)) return file;
  const ext = file.name.split(".").pop();
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  return new File([file], `pasted-${stamp}.${ext}`, { type: file.type });
}

/** A size as people read it. */
function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** Whether the conversation shown is a sub agent's session, read only (subagents U6). */
const ReadOnly = createContext(false);

/** Whether `part` is a sub agent's proposal or change whose block is in `parts`, which shows it. */
function underBlock(part: Part, parts: readonly Part[]): boolean {
  const id = subagentOf(part);
  return id !== null && parts.some((p) => p.kind === "tool" && p.id === id && p.name === "start_subagent");
}

/** What a proposal is, for its line in a read-only session (subagents U4). */
export function proposalLine(p: Part): string {
  switch (p.kind) {
    case "proposal": {
      const x = p.proposal;
      const title = typeof x.title === "string" && x.title ? x.title : "note_id" in x ? x.note_id : "Untitled";
      return `${HEADS[x.op]} ${title}`;
    }
    case "download":
      return p.proposal.source === "browser" ? `Proposed save of ${p.proposal.path ?? ""} from the browser box` : `Proposed download of ${p.proposal.filename}`;
    case "type_proposal":
      return p.proposal.op === "delete" ? `Proposed delete of type ${p.proposal.name}` : p.proposal.exists ? `Proposed change to type ${p.proposal.name}` : `Proposed new type ${p.proposal.name}`;
    case "taxonomy_proposal":
      return p.proposal.op === "delete" ? `Proposed delete of taxonomy ${p.proposal.name}` : p.proposal.exists ? `Proposed change to taxonomy ${p.proposal.name}` : `Proposed new taxonomy ${p.proposal.name}`;
    case "agent_proposal":
      return p.proposal.op === "delete" ? `Proposed delete of agent ${p.proposal.name}` : p.proposal.exists ? `Proposed change to agent ${p.proposal.name}` : `Proposed new agent ${p.proposal.name}`;
    case "memory_proposal":
      return p.proposal.op === "create" ? `Proposed a new memory of ${p.proposal.memory_agent}` : p.proposal.op === "change" ? `Proposed a change to a memory of ${p.proposal.memory_agent}` : `Proposed forgetting a memory of ${p.proposal.memory_agent}`;
    case "skill_proposal":
      return `Proposed ${skillWhat(p.proposal)}`;
    case "tool_proposal":
      return `Proposed ${toolWhat(p.proposal)}`;
    default:
      return "Proposed a change";
  }
}

/** Where a proposal stands, for its line in a read-only session. */
const STANDS: Record<ProposalState, string> = {
  open: "waiting for the user in the chat that started this sub agent",
  applying: "being applied",
  applied: "applied",
  discarded: "discarded",
  failed: "failed",
  stale: "can no longer apply",
};

/** A proposal in a sub agent's session (subagents U4): what it is and where it stands, with no Apply or Discard. */
function ReadOnlyProposal(props: { part: Part }) {
  const state = () => ("state" in props.part ? props.part.state : "open");
  const error = () => ("error" in props.part ? props.part.error : undefined);
  return (
    <p class="ai-proposal ai-proposal-read-only" role="status">
      {proposalLine(props.part)}: {STANDS[state()]}
      {error() ? ` (${error()})` : ""}
    </p>
  );
}

/** One part of an answer: text, a tool step or a sub agent's block, a card, a notice. */
function PartView(props: { part: Part; parts: readonly Part[]; turn: number; index: number }) {
  const readOnly = useContext(ReadOnly);
  const proposal = () => props.part.kind !== "applied" && "proposal" in props.part;
  return (
    <Show when={!(readOnly && proposal())} fallback={<ReadOnlyProposal part={props.part} />}>
      <Switch>
        <Match when={props.part.kind === "text" && props.part}>{(p) => <Markdown text={p().text} />}</Match>
        <Match when={props.part.kind === "tool" && props.part}>
          {(p) => (
            <Switch fallback={<ToolStep part={p()} />}>
              <Match when={p().name === "start_subagent"}>
                <SubagentBlock part={p()} parts={props.parts} turn={props.turn} />
              </Match>
              <Match when={!p().isError && p().draft}>{(d) => <DraftCard draft={d()} />}</Match>
            </Switch>
          )}
        </Match>
        <Match when={props.part.kind === "proposal" && props.part}>{(p) => <ProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "type_proposal" && props.part}>{(p) => <TypeProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "taxonomy_proposal" && props.part}>{(p) => <TaxonomyProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "agent_proposal" && props.part}>{(p) => <AgentProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "memory_proposal" && props.part}>{(p) => <MemoryProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "skill_proposal" && props.part}>{(p) => <SkillProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "tool_proposal" && props.part}>{(p) => <ToolProposalCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "applied" && props.part}>{(p) => <AppliedCard applied={p().applied} />}</Match>
        <Match when={props.part.kind === "download" && props.part}>{(p) => <DownloadCard part={p()} turn={props.turn} index={props.index} />}</Match>
        <Match when={props.part.kind === "notice" && props.part}>{(p) => <Notice part={p()} />}</Match>
        <Match when={props.part.kind === "compaction" && props.part}>
          {(p) => (
            <details class="ai-compaction" data-testid="compaction">
              <summary>
                {p().reason === "command" ? "Compacted" : "Compacted on its own"}: the model reads this summary in place of the
                conversation above
              </summary>
              <Markdown text={p().summary} />
            </details>
          )}
        </Match>
      </Switch>
    </Show>
  );
}

/** Where sub agent session `thread` of agent `agent` opens, read only (subagents U6). */
export function subagentHref(agent: string, thread: string): string {
  return `/subagent/${encodeURIComponent(agent)}/${encodeURIComponent(thread)}`;
}

/** Seconds as people read them: 42s, 3m 05s. */
function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

/** What a sub agent's block says of where it stands. */
const SUB_STATUS: Record<string, string> = { done: "done", failed: "failed", stopped: "stopped" };

/**
 * A `start_subagent` step (subagents U6): "Sub agent: <task>" or "Delegated to <agent>: <task>",
 * its live status with the tool it runs and its round, the time it has taken, its result once
 * done, and the proposals it raised. The title opens its session beside the chat, read only;
 * Stop stops it alone.
 */
function SubagentBlock(props: { part: Extract<Part, { kind: "tool" }>; parts: readonly Part[]; turn: number }) {
  const input = () => (props.part.input && typeof props.part.input === "object" && !Array.isArray(props.part.input) ? props.part.input : {}) as Record<string, Json>;
  const task = () => (typeof input().task === "string" ? (input().task as string) : "");
  const delegated = () => props.part.sub?.delegated ?? (typeof input().agent === "string" && input().agent ? (input().agent as string) : null);
  const label = () => {
    const d = delegated();
    if (!d) return null;
    const a = agents().find((x) => x.name === d);
    return a ? agentLabel(a) : d;
  };
  const sub = () => props.part.sub;
  const status = () => {
    if (!props.part.done) return "running";
    const s = sub()?.status;
    return s && s !== "running" ? s : props.part.isError ? "failed" : "done";
  };
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  const took = () => {
    const s = sub();
    if (s?.ms !== undefined && status() !== "running") return elapsed(s.ms);
    if (typeof s?.started_ms !== "number") return null;
    return elapsed((s.ended_ms ?? (status() === "running" ? now() : s.started_ms)) - s.started_ms);
  };
  const doing = () => {
    const s = sub();
    const round = s?.rounds ? `round ${s.rounds}` : "starting";
    return s?.tool ? `${activeLabel(s.tool, null)}, ${round}` : round;
  };
  const href = () => {
    const s = sub();
    return s?.thread && s.agent ? subagentHref(s.agent, s.thread) : null;
  };
  const mine = () =>
    props.parts.flatMap((p, i) => (subagentOf(p) === props.part.id ? [i] : []));
  const head = () => (label() ? `Delegated to ${label()}: ${task()}` : `Sub agent: ${task()}`);
  return (
    <section class={`ai-sub ai-sub-${status()}`} aria-label={head()}>
      <p class="ai-sub-head">
        <TypeIcon name="bot" size={14} />{" "}
        <Show when={href()} fallback={<strong>{head()}</strong>}>
          {(h) => (
            <a
              class="ai-sub-open"
              href={h()}
              title="Open this sub agent's session beside the chat"
              onClick={(e) => {
                if (wantsNewTab(e)) return;
                e.preventDefault();
                openBeside(paneAt(e.currentTarget), h(), (path) => path.startsWith("/subagent/"));
              }}
            >
              {head()}
            </a>
          )}
        </Show>
      </p>
      <p class="ai-sub-status muted">
        <span role="status">{status() === "running" ? `Running: ${doing()}` : SUB_STATUS[status()]}</span>
        <Show when={took()}>{(t) => <span class="ai-sub-time"> · {t()}</span>}</Show>
        <Show when={status() !== "running" && sub()?.rounds !== undefined}>
          <span> · {sub()!.rounds === 1 ? "1 round" : `${sub()!.rounds} rounds`}</span>
        </Show>
        <Show when={status() === "running" && sub()?.turn}>
          {(turn) => (
            <>
              {" "}
              <button type="button" class="link" onClick={() => void api.aiTurnStop(turn()).catch(() => undefined)}>
                Stop
              </button>
            </>
          )}
        </Show>
      </p>
      <Show when={props.part.isError && props.part.message}>
        <p class="muted">{props.part.message}</p>
      </Show>
      <Show when={status() !== "running" && sub()?.answer}>
        {(a) => (
          <details class="ai-sub-result">
            <summary>Result</summary>
            <Markdown text={a()} />
          </details>
        )}
      </Show>
      <For each={mine()}>{(i) => <PartView part={props.parts[i]!} parts={props.parts} turn={props.turn} index={i} />}</For>
    </section>
  );
}

/**
 * How near the end, in pixels, still counts as at the end: scrolling up by more stops following
 * the conversation, and scrolling back down to within it follows again.
 */
const AT_END_PX = 4;

/**
 * A conversation as the panel and the agent screen show it: its turns, with streaming text, tool
 * steps, proposal cards, Apply all, applied cards and notices, the activity line, and the box to
 * ask in, with the provider and model picker and Send at its foot. Enter sends; Shift+Enter
 * starts a new line. `noteId` is the note open beside it, if any. While it shows a thread, that thread has
 * no "New reply" (agents G17).
 */
export function Conversation(props: {
  session: ChatSession;
  noteId?: () => string;
  empty?: JSX.Element;
  placeholder?: string;
  /** A sub agent's session (subagents U6): no box to ask in, no Edit or Fork, and proposals shown as they stand, with Stop while it runs. */
  readOnly?: boolean;
}) {
  const s = props.session;
  const chat = s.chat;
  const [draft, setDraft] = createSignal("");
  let list: HTMLOListElement | undefined;
  let scroller: HTMLDivElement | undefined;
  createEffect(
    on(
      () => chat.thread,
      (id) => {
        if (id) onCleanup(viewThread(id));
      },
    ),
  );
  // The messages scroll on their own above the message box, which is never part of the scroll.
  // At the end of the conversation, keep the end in view as the answer grows and as the box
  // grows (the activity line, a taller draft), which shrinks the messages' area; scrolled up,
  // leave the view alone.
  // Follow mode: true while the view is at the end, which keeps it there as the answer grows.
  let pinned = true;
  onMount(() => {
    const el = scroller;
    if (!el || !list || typeof ResizeObserver === "undefined") return;
    const atEnd = () => el.scrollHeight - el.scrollTop - el.clientHeight <= AT_END_PX;
    pinned = atEnd();
    // Only scrolling up stops following: the box growing or the answer arriving moves the end away
    // from the view without the view moving, which is no reason to stop.
    let last = el.scrollTop;
    const onScroll = () => {
      const top = el.scrollTop;
      if (atEnd()) pinned = true;
      else if (top < last - 1) pinned = false;
      last = top;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    const grown = new ResizeObserver(() => {
      if (pinned) el.scrollTop = el.scrollHeight;
    });
    grown.observe(list);
    grown.observe(el);
    onCleanup(() => {
      grown.disconnect();
      el.removeEventListener("scroll", onScroll);
    });
  });
  // Files pasted, dropped or picked into the box, each uploaded as an asset at once.
  const [files, setFiles] = createSignal<Upload[]>([]);
  let picker: HTMLInputElement | undefined;
  const uploading = () => files().some((f) => f.state === "uploading");
  const ready = () => files().flatMap((f) => (f.state === "ready" && f.asset ? [f.asset] : []));
  function attach(list: FileList | File[] | null | undefined) {
    for (const file of Array.from(list ?? [])) {
      const key = ++uploads;
      const named = pastedName(file);
      setFiles((all) => [...all, { key, filename: named.name, size: named.size, state: "uploading" }]);
      const set = (patch: Partial<Upload>) => setFiles((all) => all.map((u) => (u.key === key ? { ...u, ...patch } : u)));
      api.uploadAsset(named).then(
        (a) => set({ state: "ready", asset: { id: a.id, filename: a.filename, size: a.size } }),
        (e: unknown) => set({ state: "failed", error: e instanceof Error ? e.message : String(e) }),
      );
    }
  }
  // Messages sent while a turn runs wait here, and go together as one once it ends.
  const [queued, setQueued] = createSignal<{ text: string; attachments: Attachment[] }[]>([]);
  const [problem, setProblem] = createSignal<string | null>(null);
  let box: HTMLTextAreaElement | undefined;
  // Another session shown drops the queue; a new chat getting its thread keeps it.
  createEffect(
    on(
      () => chat.thread,
      (now, was) => {
        if (was && now !== was) setQueued([]);
      },
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => chat.streaming,
      (now, was) => {
        if (!was || now || queued().length === 0) return;
        const all = queued();
        setQueued([]);
        const text = all.map((q) => q.text.trim()).filter(Boolean).join("\n\n");
        void s.ask(text, props.noteId?.() || undefined, all.flatMap((q) => q.attachments));
      },
    ),
  );

  /** Put `text` and `attachments` in the box, after what it holds, to edit and send again. */
  function intoBox(text: string, attachments: Attachment[]) {
    setDraft((d) => [d.trim(), text.trim()].filter(Boolean).join("\n\n"));
    setFiles((all) => [...all, ...attachments.map((a) => ({ key: ++uploads, filename: a.filename, size: a.size, state: "ready" as const, asset: a }))]);
    queueMicrotask(() => box?.focus());
  }

  /** The queued messages back in the box, out of the queue. */
  function editQueued() {
    const all = queued();
    setQueued([]);
    for (const q of all) intoBox(q.text, q.attachments);
  }

  async function editTurn(t: number) {
    if (!confirm("Edit this message? It and everything after it leave the conversation.")) return;
    setProblem(null);
    try {
      const got = await s.editFrom(t);
      if (got) intoBox(got.text, got.attachments);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    }
  }

  async function forkTurn(t: number) {
    setProblem(null);
    try {
      const got = await s.forkFrom(t);
      if (got) intoBox(got.text, got.attachments);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    }
  }

  async function send() {
    const text = draft();
    const attached = ready();
    if ((!text.trim() && attached.length === 0) || uploading()) return;
    setDraft("");
    setFiles([]);
    if (chat.streaming) {
      setQueued((q) => [...q, { text, attachments: attached }]);
      return;
    }
    // In an app the files reach the home server, which runs the AI, by sync.
    if (attached.length) await attachmentsSynced();
    void s.ask(text, props.noteId?.() || undefined, attached);
    // Sending follows the conversation again.
    pinned = true;
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
  }
  return (
    <ChatContext.Provider value={s}>
      <ReadOnly.Provider value={props.readOnly === true}>
      <div class="ai-conversation" classList={{ "ai-read-only": props.readOnly === true }}>
      <div class="ai-scroll" ref={scroller} data-testid="ai-scroll">
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
                      // A sub agent's proposals and changes stand under its block (subagents U4).
                      <Show when={!underBlock(part, turn.parts)}>
                        <PartView part={part} parts={turn.parts} turn={t()} index={i()} />
                      </Show>
                    )}
                  </For>
                  <Show when={!props.readOnly}>
                    <ApplyAll turn={t()} parts={turn.parts} />
                  </Show>
                </>
              }>
                {/* `/update-skills` shows as a compact notice, not a bubble (skills S6). */}
                <Show when={turn.notice}>
                  {(n) => (
                    <p class="ai-notice ai-skills-updated" role="status">
                      {n()}
                    </p>
                  )}
                </Show>
                {/* What the user did with proposals since the last message, sent ahead of it (ai D29). */}
                <Show when={turn.outcomes?.length}>
                  <details class="ai-outcomes">
                    <summary>Applied changes</summary>
                    <ul>
                      <For each={turn.outcomes}>{(line) => <li>{line}</li>}</For>
                    </ul>
                  </details>
                </Show>
                <Show when={!turn.notice && turn.text}>
                  <p class="ai-question">{turn.text}</p>
                </Show>
                <Show when={turn.at !== undefined && !turn.notice && !chat.streaming && !props.readOnly}>
                  <div class="ai-question-actions">
                    <button type="button" class="link" aria-label="Edit this message" title="Edit this message: it and everything after it leave the conversation" onClick={() => void editTurn(t())}>
                      Edit
                    </button>
                    <Show when={chat.thread}>
                      <button type="button" class="link" aria-label="Fork from this message" title="Start a new chat with everything above this message, and this message to edit" onClick={() => void forkTurn(t())}>
                        Fork
                      </button>
                    </Show>
                  </div>
                </Show>
                <Show when={turn.attachments?.length}>
                  <ul class="ai-attachments ai-sent-attachments" aria-label="Attached files">
                    <For each={turn.attachments}>
                      {(f) => (
                        <li class="ai-attachment">
                          <a href={`/assets/${f.id}`}>{f.filename}</a>
                          <span class="muted">{sizeText(f.size)}</span>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </Show>
            </li>
          )}
        </For>
      </ol>
      </div>
      <Show when={props.readOnly}>
        <div class="ai-form ai-form-read-only">
          <div class="ai-form-status">
            <Show when={chat.streaming && chat.activity}>
              <ActivityLine />
            </Show>
          </div>
          <Show when={chat.streaming}>
            <div class="ai-form-foot">
              <button type="button" onClick={s.stop}>
                Stop
              </button>
            </div>
          </Show>
        </div>
      </Show>
      <Show when={!props.readOnly}>
      <form
        class="ai-form"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        {/* Inside the form, which stays at the panel's foot, so the line sits right above the box. */}
        <div class="ai-form-status">
          <Show when={chat.streaming && chat.activity}>
            <ActivityLine />
          </Show>
          <Show when={chat.context}>{(c) => <ContextMeter use={c()} />}</Show>
        </div>
        <Show when={problem()}>
          <p class="error" role="alert">
            {problem()}
          </p>
        </Show>
        <Show when={queued().length > 0}>
          <section class="ai-queue" aria-label="Queued messages">
            <header>
              <span class="muted">Queued, sent when the answer ends</span>
              <button type="button" class="link" onClick={editQueued}>
                Edit
              </button>
              <button type="button" class="link" onClick={() => setQueued([])}>
                Clear
              </button>
            </header>
            <ol>
              <For each={queued()}>
                {(q) => (
                  <li>
                    {q.text.trim() || "(files only)"}
                    <Show when={q.attachments.length}>
                      <span class="muted"> · {q.attachments.length === 1 ? "1 file" : `${q.attachments.length} files`}</span>
                    </Show>
                  </li>
                )}
              </For>
            </ol>
          </section>
        </Show>
        <Show when={files().length > 0}>
          <ul class="ai-attachments" aria-label="Attached files">
            <For each={files()}>
              {(f) => (
                <li class={`ai-attachment ai-attachment-${f.state}`} title={f.error ?? f.filename}>
                  <span class="ai-attachment-name">{f.filename}</span>
                  <span class="muted">{f.state === "uploading" ? "Uploading…" : f.state === "failed" ? "Failed" : sizeText(f.size)}</span>
                  <button
                    type="button"
                    class="link"
                    aria-label={`Remove ${f.filename}`}
                    onClick={() => setFiles((all) => all.filter((u) => u.key !== f.key))}
                  >
                    ×
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <textarea
          ref={box}
          onPaste={(e) => {
            // Pasted files, such as a screenshot, are attached; pasted text goes in as usual.
            const pasted = e.clipboardData?.files;
            if (pasted && pasted.length > 0) {
              e.preventDefault();
              attach(pasted);
            }
          }}
          onDragOver={(e) => {
            if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
          }}
          onDrop={(e) => {
            if (!e.dataTransfer?.files.length) return;
            e.preventDefault();
            attach(e.dataTransfer.files);
          }}
          aria-label="Ask AI"
          aria-keyshortcuts="Enter"
          title="Enter sends, Shift+Enter starts a new line"
          placeholder={props.placeholder ?? "Ask about your notes"}
          rows={3}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            // Enter sends and Shift+Enter starts a new line; Enter that ends an IME composition
            // (keyCode 229 in some browsers) only commits the composed text.
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
              e.preventDefault();
              send();
            }
          }}
        />
        {/* The model the next turn uses sits at the box's foot, beside Send. */}
        <div class="ai-form-foot">
          <input
            ref={picker}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              attach(e.currentTarget.files);
              e.currentTarget.value = "";
            }}
          />
          <button type="button" class="ai-attach" aria-label="Attach files" title="Attach files, or paste or drop them in the box" onClick={() => picker?.click()}>
            📎
          </button>
          <ModelPicker session={s} />
          <Show
            when={chat.streaming}
            fallback={
              <button type="submit" class="primary" disabled={(!draft().trim() && ready().length === 0) || uploading()}>
                Send
              </button>
            }
          >
            <Show when={draft().trim() || ready().length > 0}>
              <button type="submit" disabled={uploading()} title="Queue it: sent when the answer ends">
                Queue
              </button>
            </Show>
            <button type="button" onClick={s.stop}>
              Stop
            </button>
          </Show>
        </div>
      </form>
      </Show>
      </div>
      </ReadOnly.Provider>
    </ChatContext.Provider>
  );
}

/** The provider and model picker, starting from the agent's own when the conversation's agent names them. */
export function ModelPicker(props: { session: ChatSession }) {
  return (
    <Show when={ownsModel(props.session.chat.agent) ? props.session.chat.agent : null} keyed fallback={<Picker session={props.session} />}>
      {(name) => <OwnModelPicker session={props.session} agent={name} />}
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

/** Where thread `id` of agent `agent` opens in the agent screen's chat. */
export function threadHref(agent: string, id: string): string {
  return `/agents/${encodeURIComponent(agent)}?thread=${encodeURIComponent(id)}`;
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
  // A memo, so another agent added or changed elsewhere does not close the menu or reread the list.
  const listed = createMemo(() => agents().some((a) => a.name === chat.agent));
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
      // Sub agent sessions are listed only under their parent (subagents U5).
      setThreads(got.threads.filter((t) => !isSubSession(t.origin)));
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
            Ask about your notes. Answers come from {accountShown(chat)}; the notes it reads are sent there. Edits are proposals you apply, unless the agent's permissions let them go ahead without asking.
          </p>
        }
      />
    </div>
  );
}
