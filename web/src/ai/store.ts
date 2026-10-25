// AI conversations (ai D10, D14, D23; agents G6, G7, G8): what a conversation shows, and the
// neutral transcript of its turns. Each conversation is a thread of an agent, the built-in
// Assistant unless another is chosen: the server keeps the thread as a note, reads its
// transcript for each turn, so a turn sends only the new message, and keeps each proposal's
// outcome, so a reopened thread shows its pending cards. Against a server without threads the
// conversation lives in this tab's memory and each turn sends the whole transcript. The chat
// runs on the home server, which holds the credentials and runs the note tools. Every client
// starts a turn with POST /api/ai/chat/turns, and the turn runs there until it ends or is
// stopped, whoever reads it (agents G17): switching threads or agents, closing the panel or the
// tab never stops it. Opening a thread whose turn runs and that this tab does not follow already
// reads that turn again from its first event, over the thread as it stood before the turn. In the web app the events stream
// from GET /api/ai/chat/turns/{turn}/events?after=<n>; in the desktop and Android apps, whose
// webviews get whole bodies from the embedded server, they are polled from the embedded server's
// GET /api/app/ai/turns/{turn}?after=<n>, which reads the turn's stream from the home server.
// Edits come
// back as proposals the user applies with POST /api/ai/apply, on the home server in every
// client, or, for the kinds the agent lets go ahead without asking, as `applied` events for
// changes already made. In an app such a change reaches the device's copy with a sync round,
// which the panel asks for. A thread's conversation is one store per thread id, shared by every
// view in the tab that shows it, with the one subscription to its turn's events (agents G18):
// the right sidebar's panel and the agent screen's chat are views made with
// `createChatSession`, each choosing its own thread. A conversation goes once no view holds it
// and its turn has ended, so one a view left while its turn ran is found as it stands.

import { createContext, createRoot, createSignal, untrack, useContext } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { ASSISTANT, agents, agentsChanged, memoriesChanged, refreshAgents } from "../agents";
import {
  type AgentThread,
  type AiApplied,
  type AppInfo,
  type Json,
  type NoteDoc,
  type SkillOp,
  ApiError,
  type StoredProposal,
  type StreamEvent,
  api,
  streamTurn,
} from "../api";
import { noteChangedElsewhere, noteWritten } from "../changes";
import { appInfo } from "../inApp";
import { reloadDefinitions } from "../session";
import { skillsChanged } from "../skills";
import { refreshSync } from "../sync/store";
import { bumpTurns, turnEnded } from "./running";

/** One piece of a transcript message, as the server's `ai::Content`. */
export type Content =
  | { type: "text"; text: string }
  | { type: "tool_call"; id: string; name: string; input: Json }
  | { type: "tool_result"; id: string; content: string; is_error?: boolean; parts?: ResultPart[] }
  | { type: "notice"; kind: string; text: string };

/** A tool result's part beyond its text (ai D19): an image by asset id, never its bytes. */
export type ResultPart = { type: "text"; text: string } | { type: "image"; asset_id: string; media_type: string };

/** The address a `fetch_url` or `download_asset` step read, from its `tool_result` summary (ai D27). */
export interface WebRef {
  url: string;
  host?: string;
  media_type?: string;
  /** `html`, `text`, `pdf`, `image`, `metadata` or `download`. */
  kind?: string;
  title?: string;
}

/**
 * What a browser step did (browser B5), from its `tool_result` summary: a `browser_run`'s command
 * and output, a `browser_file`'s file (an image with a small thumbnail as a data URL, since the
 * image is not in the vault), or a `browser_save`'s file and asset.
 */
export interface BrowserRef {
  /** `run`; `image`, `text` or `metadata` for a read; `save`. */
  kind: string;
  command?: string;
  exit?: number;
  ms?: number;
  timed_out?: boolean;
  stdout?: string;
  stderr?: string;
  path?: string;
  media_type?: string;
  size?: number;
  thumbnail?: string;
  filename?: string;
  asset_id?: string;
}

/** The asset a `read_asset` step read, from its `tool_result` summary. */
export interface AssetRef {
  id: string;
  filename: string;
  media_type: string;
  size: number;
  /** `image`, `pdf`, `text` or `metadata`: what the model got. */
  kind: string;
}

/** An asset a `find_assets` step found, from its `tool_result` summary (ai D29). */
export interface FoundAsset {
  id: string;
  filename: string;
}

/** The Outbox item a `draft_email`, `draft_sms` or `draft_event` step made (outbox X9). */
export interface DraftRef {
  note_id: string;
  /** `email`, `sms` or `event`. */
  kind: string;
  /** `draft`, or `ready` for an agent that sends without review (outbox X14). */
  status: string;
  title: string;
  /** Who it goes to, by name where a person note named them. */
  to: string[];
}

/** One transcript message. `provider_state` is the provider's own record, sent back as it came. */
export interface ChatMessage {
  role: "user" | "assistant";
  content: Content[];
  provider_state?: Json;
}

export interface NoteRef {
  id: string;
  title: string;
}

/** One change a proposal makes: a field's value, or a top-level block's content. */
export interface ProposalOp {
  op: "field" | "block";
  name: string;
  before: Json;
  after: Json;
}

/** What a note proposal does (ai D20). A proposal without `op` is an `update`. */
export type NoteOp = "update" | "create" | "archive" | "unarchive" | "delete";

/** A note change the model proposed, checked by the server but not written (ai D14, D20). */
export type Proposal =
  | {
      op: "update";
      id: string;
      note_id: string;
      title: Json;
      base_version: string;
      before: NoteDoc;
      after: NoteDoc;
      ops: ProposalOp[];
    }
  | { op: "create"; id: string; type: string; title: Json; after: NoteDoc }
  | { op: "archive" | "unarchive" | "delete"; id: string; note_id: string; title: Json; base_version: string };

/** `stale`: the note changed since the proposal, so it cannot apply; only Discard is left. */
export type ProposalState = "open" | "applying" | "applied" | "discarded" | "failed" | "stale";

/** A type file the model proposed, checked by the server but not written (ai D18). */
export interface TypeProposal {
  id: string;
  name: string;
  /** Whether the type existed when proposed; a new name proposes a new type. */
  exists: boolean;
  /** `delete` for a proposed delete of the type; absent for a new or changed type. */
  op?: "delete";
  /** `type` on a proposed delete, so the apply knows it for a type's. */
  kind?: "type";
  /** The type file as stored when proposed; null for a new type. */
  before: string | null;
  /** The new type file; null for a delete. */
  after: string | null;
  /** Warnings the check found, such as an attribute this version ignores. */
  issues: { at?: string; message: string; severity?: string }[];
}

/** As a note proposal's: `stale` when the type changed since the proposal. */
export type TypeProposalState = ProposalState;

/** A term renamed or moved by a taxonomy proposal, old path to new (ai D31). */
export interface TermRename {
  from: string;
  to: string;
}

/** A taxonomy change the model proposed, checked by the server but not written (ai D31). */
export type TaxonomyProposal =
  | {
      id: string;
      kind: "taxonomy";
      op: "create" | "change";
      name: string;
      /** Whether the taxonomy existed when proposed; a new name proposes a new taxonomy. */
      exists: boolean;
      /** The taxonomy file as stored when proposed; null for a new taxonomy. */
      before: string | null;
      after: string;
      renames: TermRename[];
      issues: { at?: string; message: string; severity?: string }[];
      /** How many notes the renames rewrite. */
      affected: number;
    }
  | { id: string; kind: "taxonomy"; op: "delete"; name: string; before: string };

/**
 * An agent change the model proposed, checked by the server against the agent write and against
 * the proposing agent's own permissions, but not written (agents G20).
 */
export type AgentProposal =
  | {
      id: string;
      kind: "agent";
      op: "create" | "change";
      name: string;
      label?: string;
      /** Whether the agent existed when proposed; a new name proposes a new agent. */
      exists: boolean;
      /** The agent file as stored when proposed; null for a new agent. */
      before: string | null;
      after: string;
      issues: { at?: string; message: string; severity?: string }[];
      /** The agent that proposed it, which the server checks the change against again. */
      proposer?: string;
    }
  | { id: string; kind: "agent"; op: "delete"; name: string; label?: string; before: string; proposer?: string };

/**
 * A change to another agent's memories the model proposed (agents G21), checked by the server
 * against the proposing agent's reach, but not written: a new memory, a change with the title and
 * text before, or forgetting one.
 */
export interface MemoryProposal {
  id: string;
  kind: "agent_memory";
  op: "create" | "change" | "delete";
  /** The agent whose memory it is. */
  memory_agent: string;
  agent_label?: string;
  /** The memory; null for a new one. */
  memory_id: string | null;
  base_version?: string | null;
  /** The title and text after the change; for a delete, the memory that goes. */
  title: string;
  text: string;
  before: { title: string; text: string } | null;
  /** The agent that proposed it, which the server checks the change against again. */
  proposer?: string;
}

/**
 * A skill change the model proposed (skills S9), checked by the server as the skill writes check
 * it, but not written: a new `SKILL.md` or a change to one, a file written, a file or folder
 * deleted, or the whole skill deleted. `before` and `after` are text when the file is text;
 * a binary file shows its `size` and `media_type`.
 */
export interface SkillProposal {
  id: string;
  kind: "skill";
  op: "create" | "change" | "file" | "file_delete" | "delete";
  name: string;
  /** The file, for `file` and `file_delete`; `SKILL.md` for `create` and `change`. */
  path?: string;
  /** Whether the file (or for `create` and `change`, the skill) existed when proposed. */
  exists?: boolean;
  before: string | null;
  after: string | null;
  issues: { at?: string; message: string; severity?: string }[];
  binary?: boolean;
  size?: number;
  media_type?: string | null;
  before_size?: number | null;
  before_media_type?: string | null;
  /** The paths a delete removes. */
  files?: string[];
  /** For a `file` change to a skill not created yet: false. */
  skill_exists?: boolean;
  proposer?: string;
}

/** What the card says when the skill changed after the proposal (skills S9). */
export const SKILL_CHANGED = "This skill changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What the card says when the memory changed after the proposal (agents G21). */
export const MEMORY_CHANGED = "This memory changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What the card says when the agent changed after the proposal (agents G20). */
export const AGENT_CHANGED = "This agent changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What the card says when the taxonomy changed after the proposal (ai D31). */
export const TAXONOMY_CHANGED = "This taxonomy changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What the card says when the type changed after the proposal (ai D18). */
export const TYPE_CHANGED = "This type changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/**
 * A change the AI made itself, its confirmation turned off in Settings > AI (ai D22): a note's,
 * with its new version (for a delete, the version that went to the trash), or a type's.
 */
/** A web page a `web_search` step found, from its `tool_result` summary (ai D26). */
export interface WebLink {
  title: string;
  url: string;
}

export type Applied =
  | { id: string; op: NoteOp; note_id: string; title: Json; version: string }
  | { id: string; op: "type_change" | "type_create" | "type_delete"; type_name: string }
  | { id: string; op: "taxonomy_change" | "taxonomy_create" | "taxonomy_delete"; taxonomy_name: string; notes?: number }
  | { id: string; op: "agent_change" | "agent_create" | "agent_delete"; agent_name: string }
  | { id: string; op: "agent_memory_create" | "agent_memory_change" | "agent_memory_delete"; memory_id: string; memory_agent: string; version?: string }
  | { id: string; op: SkillOp; skill_name: string; path?: string };

/** A file the model asked to download into the vault as an asset (ai D27). */
export interface DownloadProposal {
  id: string;
  op: "download";
  /** `browser` for a file from the browser box (browser B5), which has `path` and no `url`. */
  source?: "browser";
  path?: string;
  url: string;
  filename: string;
  media_type: string | null;
  size: number | null;
}

/** The asset a download stored. */
export interface DownloadedAsset {
  asset_id: string;
  filename: string;
  media_type?: string;
  size?: number;
}

export type Part =
  | { kind: "text"; text: string }
  /** A download (ai D27): proposed and waiting, or stored, by Apply or by the AI itself (`auto`). */
  | { kind: "download"; proposal: DownloadProposal; state: ProposalState; error?: string; asset?: DownloadedAsset; auto?: boolean }
  | { kind: "applied"; applied: Applied }
  | {
      kind: "tool";
      id: string;
      name: string;
      input: Json;
      done: boolean;
      isError: boolean;
      notes: NoteRef[];
      message?: string;
      asset?: AssetRef;
      links?: WebLink[];
      web?: WebRef;
      /** The assets a `find_assets` step found (ai D29). */
      assets?: FoundAsset[];
      /** The Outbox item a draft step made (outbox X9). */
      draft?: DraftRef;
      /** What a browser step did (browser B5). */
      browser?: BrowserRef;
    }
  | { kind: "proposal"; proposal: Proposal; state: ProposalState; error?: string; created?: string }
  | { kind: "type_proposal"; proposal: TypeProposal; state: TypeProposalState; error?: string }
  | { kind: "taxonomy_proposal"; proposal: TaxonomyProposal; state: ProposalState; error?: string }
  | { kind: "agent_proposal"; proposal: AgentProposal; state: ProposalState; error?: string }
  | { kind: "memory_proposal"; proposal: MemoryProposal; state: ProposalState; error?: string }
  | { kind: "skill_proposal"; proposal: SkillProposal; state: ProposalState; error?: string }
  | { kind: "notice"; notice: string; text: string; category?: string | null }
  /** Where the thread was compacted: the model reads `summary` in place of what came before. */
  | { kind: "compaction"; summary: string; reason: string };

export interface Turn {
  role: "user" | "assistant";
  /** A user turn's question. */
  text?: string;
  /**
   * A user turn's lines on the proposals applied or discarded since the message before (ai D29),
   * sent ahead of the question and shown collapsed.
   */
  outcomes?: string[];
  /**
   * A user turn shown as a compact notice in place of a bubble: `/update-skills` (skills S6),
   * "Skills updated (<n>)" once the server took the new list.
   */
  notice?: string;
  /** A user turn's attached files. */
  attachments?: Attachment[];
  /** A user turn's place in the transcript: the index of its message, where Edit and Fork cut. */
  at?: number;
  /** An assistant turn's answer, tool steps, proposals and notices, in the order they came. */
  parts: Part[];
}

/** What typed alone in the chat box takes a fresh list of the agent's skills (skills S6). */
export const UPDATE_SKILLS = "/update-skills";

/** The notice kind of the user message a `/update-skills` turn keeps. */
export const SKILLS_UPDATED = "skills_updated";

/** The user turn's notice while the server takes the new list of skills. */
export const UPDATING_SKILLS = "Updating skills…";

/**
 * What a running turn is doing now, for the activity line above the message box (ai D30): from
 * the last event, thinking before any event and after a tool result, a tool's step after its
 * `tool` event, writing after a `text` delta.
 */
export type Activity =
  | { step: "thinking" }
  | { step: "writing" }
  | { step: "compacting" }
  | { step: "tool"; name: string; input: Json };

/** A file pasted, dropped or picked into the chat box, stored in the vault as an asset. */
export interface Attachment {
  id: string;
  filename: string;
  size: number;
}

/** How the files attached to a message read to the model: one line each, under a heading. */
export function attachmentText(files: readonly Attachment[]): string {
  const lines = files.map((f) => `- ${f.filename} (asset ${f.id}, ${f.size} bytes)`);
  return `${ATTACHED_HEADING}\n${lines.join("\n")}`;
}

const ATTACHED_HEADING =
  "[Files the user attached to this message, stored in the vault as assets: read each with read_asset, and use its asset id to put it in a note.]";

/** The files an attachment text names, or null for any other text. */
export function attachmentsOf(text: string): Attachment[] | null {
  if (!text.startsWith(ATTACHED_HEADING)) return null;
  const out: Attachment[] = [];
  for (const line of text.split("\n").slice(1)) {
    const m = /^- (.*) \(asset ([0-9A-Z]+), (\d+) bytes\)$/.exec(line);
    if (m) out.push({ filename: m[1]!, id: m[2]!, size: Number(m[3]) });
  }
  return out;
}

/** What typed alone in the chat box compacts the thread now. */
export const COMPACT = "/compact";

/** The user turn's notice while the thread is compacted, and once it is. */
export const COMPACTING = "Compacting the conversation…";
export const COMPACTED = "Conversation compacted";

/** How full a thread's context was after its last model call. */
export interface ContextUse {
  used: number;
  window: number;
}

const LABELS: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", chatgpt: "ChatGPT" };

export function providerLabel(p: string | null | undefined): string {
  return (p && LABELS[p]) || p || "The provider";
}

/** The refusal notice's text: who refused and the category they gave (ai D11). */
export function refusalText(provider: string | null, model: string | null, category: string | null, explanation: string | null): string {
  let t = `${providerLabel(provider)}${model ? ` ${model}` : ""} refused this request`;
  if (category) t += ` (category: ${category})`;
  t += ". The answer above is cut off.";
  if (explanation) t += ` ${explanation}`;
  return t;
}
const appState = createRoot(() => {
  const [app, setApp] = createSignal<AppInfo | null>(null);
  return { app, setApp };
});

let asked = false;

/**
 * Reactive: the desktop or Android app the page runs in, or null in the web app and until the
 * answer is known. The panel shows in both; in an app it uses the turns form (ai D23).
 */
export function inApp(): AppInfo | null {
  if (!asked) {
    asked = true;
    void appInfo().then(
      (a) => appState.setApp(a),
      () => undefined,
    );
  }
  return appState.app();
}

/** Test-only: ask again whether the page runs in an app. */
export function forgetApp() {
  asked = false;
  appState.setApp(null);
}
const quoted = (title: Json) => (typeof title === "string" && title ? ` "${title}"` : "");

/** What a note proposal is, for an outcome line. */
function proposalWhat(p: Proposal): string {
  switch (p.op) {
    case "update":
      return `edit of note ${p.note_id}`;
    case "create":
      return `new ${p.type} note${quoted(p.title)}`;
    default:
      return `${p.op} of note ${p.note_id}`;
  }
}

/** What an applied note proposal did, for an outcome line. */
function proposalDone(p: Proposal, done: AiApplied): string {
  const version = "version" in done && done.version ? done.version : "";
  switch (p.op) {
    case "create":
      return `created note ${"note_id" in done ? done.note_id : ""}${quoted(p.title)}${version ? `, version ${version}` : ""}`;
    case "delete":
      return `delete of note ${p.note_id}: moved to the trash`;
    default:
      return `${proposalWhat(p)}${version ? ` now at version ${version}` : ""}`;
  }
}

const typeWhat = (p: TypeProposal) =>
  p.op === "delete" ? `delete of type ${p.name}` : p.exists ? `change to type ${p.name}` : `new type ${p.name}`;
const downloadWhat = (p: DownloadProposal) =>
  p.source === "browser" ? `save of ${p.path ?? ""} from the browser box` : `download of ${p.url}`;

/** What a taxonomy proposal is, for an outcome line (ai D31). */
function taxonomyWhat(p: TaxonomyProposal): string {
  if (p.op === "delete") return `delete of taxonomy ${p.name}`;
  return p.exists ? `change to taxonomy ${p.name}` : `new taxonomy ${p.name}`;
}

/** What an applied taxonomy proposal did, for an outcome line. */
function taxonomyDone(p: TaxonomyProposal, done: AiApplied): string {
  if (p.op === "delete") return `deleted taxonomy ${p.name}`;
  if (!p.exists) return `created taxonomy ${p.name}`;
  const notes = "notes" in done && typeof done.notes === "number" ? done.notes : 0;
  return `changed taxonomy ${p.name}${notes ? `, its renames moving ${notes} ${notes === 1 ? "note" : "notes"}` : ""}`;
}

/** What an agent proposal is, for an outcome line (agents G20). */
function agentWhat(p: AgentProposal): string {
  if (p.op === "delete") return `delete of agent ${p.name}`;
  return p.exists ? `change to agent ${p.name}` : `new agent ${p.name}`;
}

/** What an applied agent proposal did, for an outcome line. */
function agentDone(p: AgentProposal): string {
  if (p.op === "delete") return `deleted agent ${p.name}`;
  return p.exists ? `changed agent ${p.name}` : `created agent ${p.name}`;
}

/** What a memory proposal is, for an outcome line (agents G21). */
function memoryWhat(p: MemoryProposal): string {
  if (p.op === "delete") return `forgetting memory ${p.memory_id} of agent ${p.memory_agent}`;
  if (p.op === "change") return `change to memory ${p.memory_id} of agent ${p.memory_agent}`;
  return `new memory "${p.title}" of agent ${p.memory_agent}`;
}

/** What an applied memory proposal did, for an outcome line. */
function memoryDone(p: MemoryProposal, done: AiApplied): string {
  if (p.op === "delete") return `forgot memory ${p.memory_id} of agent ${p.memory_agent}: moved to the trash`;
  if (p.op === "change") return `changed memory ${p.memory_id} of agent ${p.memory_agent}`;
  const id = "memory_id" in done ? done.memory_id : "";
  return `created memory ${id} "${p.title}" of agent ${p.memory_agent}`;
}

/** What a skill proposal is, for an outcome line (skills S9). */
export function skillWhat(p: SkillProposal): string {
  switch (p.op) {
    case "create":
      return `new skill ${p.name}`;
    case "change":
      return `change to skill ${p.name}`;
    case "file":
      return `file ${p.path ?? ""} of skill ${p.name}`;
    case "file_delete":
      return `delete of ${p.path ?? ""} from skill ${p.name}`;
    default:
      return `delete of skill ${p.name}`;
  }
}

/** What an applied skill proposal did, for an outcome line. */
export function skillDone(p: SkillProposal, done?: AiApplied): string {
  const op = done && "skill_name" in done ? done.op : null;
  switch (p.op) {
    case "create":
    case "change":
      // The server says whether the write created the skill.
      return (op ?? (p.op === "create" ? "skill_create" : "skill_change")) === "skill_create" ? `created skill ${p.name}` : `changed skill ${p.name}`;
    case "file":
      return `wrote ${p.path ?? ""} in skill ${p.name}`;
    case "file_delete":
      return `deleted ${p.path ?? ""} from skill ${p.name}`;
    default:
      return `deleted skill ${p.name}`;
  }
}

/** The notices the server put in the turn's new messages, such as a round limit or a cut-off. */
function noticesOf(data: { messages?: unknown }): Part[] {
  if (!Array.isArray(data.messages)) return [];
  const out: Part[] = [];
  for (const m of data.messages as ChatMessage[]) {
    for (const c of m.content ?? []) {
      if (c.type === "notice" && c.kind !== "refusal") out.push({ kind: "notice", notice: c.kind, text: c.text });
    }
  }
  return out;
}

/** The activity line's step after event `e` (ai D30); undefined leaves it as it is. */
function activityAfter(e: StreamEvent, d: Record<string, unknown>): Activity | null | undefined {
  switch (e.event) {
    case "text":
      return { step: "writing" };
    case "tool":
      return { step: "tool", name: String(d.name), input: (d.input ?? null) as Json };
    case "tool_result":
    case "compacted":
    case "compact_failed":
      return { step: "thinking" };
    case "compacting":
      return { step: "compacting" };
    case "done":
    case "error":
    case "refusal":
      return null;
    default:
      return undefined;
  }
}

/**
 * Poll turn `turn`'s events after the first `after` from the embedded server, as the apps read
 * them (ai D23), until the turn's stream ends. Aborting `signal` stops reading; the turn runs on.
 */
async function pollTurn(turn: string, after: number, onEvent: (e: StreamEvent) => void, signal: AbortSignal) {
  let at = after;
  for (;;) {
    signal.throwIfAborted();
    const got = await api.aiTurnPoll(turn, at, signal);
    signal.throwIfAborted();
    for (const e of got.events) onEvent(e);
    at = got.next;
    if (got.done) return;
  }
}

/** The events that end a turn. */
const TERMINAL = new Set(["done", "error", "refusal"]);

/** How often a turn's stream that broke off is read again before the conversation gives up. */
export const REATTACH_TRIES = 5;

/** How often a thread a trigger runs on is read again until its run ends (agents G17). */
export const WATCH_MS = 2_000;

/**
 * Where the turn running on a thread began: after the last message the user asked, so the
 * messages from there on are the running turn's, which reading the turn again brings back.
 */
export function turnStart(messages: readonly ChatMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]!;
    if (m.role === "user" && !(m.content ?? []).some((c) => c.type === "tool_result")) return i + 1;
  }
  return 0;
}

/** How long the app waits for the sync round that brings a change made on the server. */
export const SETTLE_MS = 20_000;
/** How often the app asks whether that round is over. */
export const SETTLE_POLL_MS = 500;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * In an app, files the user attached reach the home server, which runs the AI, only by sync: ask
 * for a round and wait until no upload waits, or [`SETTLE_MS`] passes. In a browser, nothing.
 */
export async function attachmentsSynced(): Promise<void> {
  if (!(await appInfo().catch(() => null))) return;
  const asked = Date.now();
  try {
    await api.appSyncNow();
    for (let waited = 0; waited < SETTLE_MS; waited += SETTLE_POLL_MS) {
      const s = await api.appSync();
      if (s.state !== "syncing" && (s.last_attempt_ms ?? 0) >= asked && (s.assets_waiting ?? 0) === 0) break;
      await sleep(SETTLE_POLL_MS);
    }
  } catch {
    // The turn says so if the server does not have a file yet.
  }
}

/**
 * After a change the AI made on the home server, through Apply or itself: open views of the note
 * or type take it. In an app the change reaches the device's copy only by sync, so the app asks
 * its embedded server for a round first and waits for it to end (ai D23).
 */
export async function settle(kind: "note" | "type" | "taxonomy" | "agent") {
  if (await appInfo().catch(() => null)) {
    const asked = Date.now();
    try {
      await api.appSyncNow();
      for (let waited = 0; waited < SETTLE_MS; waited += SETTLE_POLL_MS) {
        const s = await api.appSync();
        if (s.state !== "syncing" && (s.last_attempt_ms ?? 0) >= asked) break;
        await sleep(SETTLE_POLL_MS);
      }
    } catch {
      // The sync icon shows why; the change arrives with a later round.
    }
    void refreshSync();
    if (kind !== "type" && kind !== "agent") noteChangedElsewhere(null);
  }
  // An agent change shows in the sidebar's list and any open agent screen (agents G20).
  if (kind === "agent") {
    await agentsChanged().catch(() => undefined);
    return;
  }
  if (kind !== "note") await reloadDefinitions().catch(() => undefined);
  // A taxonomy's renames rewrite the notes holding its terms (ai D31).
  if (kind !== "type") noteWritten();
}

/** What a card says when the note changed after the proposal (ai D20, D23). */
export const NOTE_CHANGED = "This note changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What a card says when the note the proposal names is gone. */
export const NOTE_GONE = "This note is no longer there, so nothing was applied.";

/** Whether a part is a proposal still waiting for the user: open, or failed and retryable. */
export function pending(p: Part): boolean {
  return (
    (p.kind === "proposal" || p.kind === "type_proposal" || p.kind === "taxonomy_proposal" || p.kind === "agent_proposal" || p.kind === "memory_proposal" || p.kind === "skill_proposal" || p.kind === "download") &&
    (p.state === "open" || p.state === "failed")
  );
}

/** Fill tool step `p` from its `tool_result` summary. */
function fillStep(p: Extract<Part, { kind: "tool" }>, raw: unknown, isError: boolean) {
  const summary = (raw ?? {}) as {
    notes?: NoteRef[];
    message?: string;
    asset?: AssetRef;
    links?: WebLink[];
    web?: WebRef;
    assets?: FoundAsset[];
    draft?: DraftRef;
    browser?: BrowserRef;
  };
  p.done = true;
  p.isError = isError;
  p.notes = (summary.notes ?? []).filter((n) => n && typeof n.id === "string").map((n) => ({ id: n.id, title: String(n.title ?? n.id) }));
  if (summary.message) p.message = summary.message;
  if (summary.asset && typeof summary.asset.id === "string") p.asset = summary.asset;
  if (Array.isArray(summary.links))
    p.links = summary.links
      .filter((l) => l && typeof l.url === "string" && /^https?:\/\//i.test(l.url))
      .map((l) => ({ url: l.url, title: String(l.title || l.url) }));
  if (summary.web && typeof summary.web.url === "string") p.web = summary.web;
  if (Array.isArray(summary.assets))
    p.assets = summary.assets.filter((a) => a && typeof a.id === "string").map((a) => ({ id: a.id, filename: String(a.filename ?? a.id) }));
  const b = summary.browser;
  if (b && typeof b === "object" && typeof b.kind === "string") {
    // Only a JPEG data URL is shown as a thumbnail: the server makes it, and nothing else is trusted as an image source.
    const thumbnail = typeof b.thumbnail === "string" && b.thumbnail.startsWith("data:image/jpeg;base64,") ? b.thumbnail : undefined;
    p.browser = { ...b, thumbnail };
  }
  const d = summary.draft;
  if (d && typeof d.note_id === "string")
    p.draft = {
      note_id: d.note_id,
      kind: String(d.kind ?? ""),
      status: String(d.status ?? ""),
      title: String(d.title ?? ""),
      to: Array.isArray(d.to) ? d.to.map(String) : [],
    };
}

/** The card of a `proposal`, `type_proposal`, `taxonomy_proposal` or `applied` event, open. */
function eventPart(event: string, d: Record<string, unknown>): Part | null {
  switch (event) {
    case "proposal":
      if (d.op === "download") return { kind: "download", proposal: d as unknown as DownloadProposal, state: "open" };
      return { kind: "proposal", proposal: { ...d, op: typeof d.op === "string" ? d.op : "update" } as unknown as Proposal, state: "open" };
    case "type_proposal":
      return { kind: "type_proposal", proposal: { ...(d as unknown as TypeProposal), issues: Array.isArray(d.issues) ? (d.issues as TypeProposal["issues"]) : [] }, state: "open" };
    case "taxonomy_proposal": {
      const p =
        d.op === "delete"
          ? (d as unknown as TaxonomyProposal)
          : ({
              ...d,
              op: d.exists ? "change" : "create",
              renames: Array.isArray(d.renames) ? d.renames : [],
              issues: Array.isArray(d.issues) ? d.issues : [],
              affected: typeof d.affected === "number" ? d.affected : 0,
            } as unknown as TaxonomyProposal);
      return { kind: "taxonomy_proposal", proposal: p, state: "open" };
    }
    case "agent_proposal": {
      const p =
        d.op === "delete"
          ? (d as unknown as AgentProposal)
          : ({ ...d, op: d.exists ? "change" : "create", issues: Array.isArray(d.issues) ? d.issues : [] } as unknown as AgentProposal);
      return { kind: "agent_proposal", proposal: p, state: "open" };
    }
    case "agent_memory_proposal":
      return { kind: "memory_proposal", proposal: d as unknown as MemoryProposal, state: "open" };
    case "skill_proposal":
      return {
        kind: "skill_proposal",
        proposal: {
          ...(d as unknown as SkillProposal),
          before: typeof d.before === "string" ? d.before : null,
          after: typeof d.after === "string" ? d.after : null,
          issues: Array.isArray(d.issues) ? (d.issues as SkillProposal["issues"]) : [],
        },
        state: "open",
      };
    case "applied": {
      if (d.op === "download") {
        const asset = d as unknown as DownloadedAsset & { url?: string; source?: string; path?: string };
        const browser = asset.source === "browser" ? { source: "browser" as const, path: asset.path ?? "" } : {};
        return {
          kind: "download",
          proposal: {
            id: String(d.id),
            op: "download",
            ...browser,
            url: asset.url ?? "",
            filename: asset.filename,
            media_type: asset.media_type ?? null,
            size: asset.size ?? null,
          },
          state: "applied",
          asset,
          auto: true,
        };
      }
      return { kind: "applied", applied: d as unknown as Applied };
    }
    default:
      return null;
  }
}

/** A stored proposal's card as a reopened thread shows it (agents G8). */
function storedPart(p: StoredProposal): Part | null {
  const part = eventPart(p.event, p.data);
  if (!part || p.status === "made" || !("state" in part)) return part;
  const state: ProposalState = p.status === "pending" ? "open" : p.status;
  const out = { ...part, state, ...(p.error ? { error: p.error } : {}) } as Part;
  if (out.kind === "proposal" && p.status === "applied" && out.proposal.op === "create" && typeof p.result?.note_id === "string") out.created = p.result.note_id;
  if (out.kind === "download" && p.status === "applied" && typeof p.result?.asset_id === "string") out.asset = p.result as unknown as DownloadedAsset;
  return out;
}

/** Whether `text` is a block of outcome lines rather than the user's own words (ai D29). */
function isOutcomeLines(text: string): boolean {
  return /^\s*\[(Applied by the user|Discarded by the user|Failed to apply):/.test(text);
}

/**
 * A stored thread as the conversation shows it: each user message with its outcome lines, and
 * each answer's text, tool steps with what they found, proposal cards as they stand, and
 * notices.
 */
export function turnsFromThread(t: AgentThread): Turn[] {
  const turns: Turn[] = [];
  const steps = t.steps ?? {};
  const proposals = t.proposals ?? [];
  const answer = (): Turn => {
    const last = turns[turns.length - 1];
    if (last?.role === "assistant") return last;
    const fresh: Turn = { role: "assistant", parts: [] };
    turns.push(fresh);
    return fresh;
  };
  const compaction = t.compaction ?? null;
  const messages = (t.messages ?? []) as ChatMessage[];
  /** Mark where the model's view starts: the summary, at the end of the turn before it. */
  const mark = () => {
    if (compaction) answer().parts.push({ kind: "compaction", summary: compaction.summary, reason: compaction.reason });
  };
  for (const [index, m] of messages.entries()) {
    if (compaction && index === compaction.upto && index > 0) mark();
    const content = m.content ?? [];
    const results = content.filter((c) => c.type === "tool_result");
    if (m.role === "user" && results.length === 0) {
      const texts = content.flatMap((c) => (c.type === "text" ? [c.text] : []));
      const lines = texts.filter(isOutcomeLines).flatMap((x) => x.split("\n").filter((l) => l.trim()));
      const files = texts.flatMap((x) => attachmentsOf(x) ?? []);
      const words = texts.filter((x) => !isOutcomeLines(x) && attachmentsOf(x) === null).join("\n\n");
      // A `/update-skills` turn shows its notice, not the text the model read (skills S6).
      const skills = content.find((c) => c.type === "notice" && c.kind === SKILLS_UPDATED);
      const notice = skills?.type === "notice" ? { text: "", notice: skills.text } : { text: words };
      turns.push({
        role: "user",
        ...notice,
        parts: [],
        at: index,
        ...(lines.length ? { outcomes: lines } : {}),
        ...(files.length ? { attachments: files } : {}),
      });
      continue;
    }
    const turn = answer();
    for (const c of content) {
      if (c.type === "text" && m.role === "assistant") turn.parts.push({ kind: "text", text: c.text });
      else if (c.type === "tool_call") {
        const step: Extract<Part, { kind: "tool" }> = { kind: "tool", id: c.id, name: c.name, input: c.input, done: false, isError: false, notes: [] };
        const got = steps[c.id];
        if (got) fillStep(step, got.summary, got.is_error === true);
        turn.parts.push(step);
      } else if (c.type === "tool_result") {
        const step = turn.parts.find((p) => p.kind === "tool" && p.id === c.id);
        if (step?.kind === "tool" && !step.done) fillStep(step, null, c.is_error === true);
        for (const p of proposals) {
          if (p.data.id !== c.id) continue;
          const part = storedPart(p);
          if (part) turn.parts.push(part);
        }
      } else if (c.type === "notice") {
        turn.parts.push({ kind: "notice", notice: c.kind, text: c.text });
      }
    }
  }
  if (compaction && compaction.upto >= messages.length && messages.length > 0) mark();
  return turns;
}

/** How a conversation is kept. */
export interface ChatOptions {
  /** The agent whose conversation it is; the built-in Assistant when none is named. */
  agent?: string;
  /**
   * Keep the conversation as a thread of the agent on the server (agents G7): the first message
   * starts one, the server keeps the transcript and the proposals' outcomes, and each turn sends
   * only the new message. A server without threads keeps the conversation in this tab instead.
   */
  threads?: boolean | "listed";
  /** Called when a turn ends, a thread starts or a proposal is applied or discarded. */
  onChange?: () => void;
}

/** What one thread's conversation shows, the same in every view of it (agents G18). */
interface ConversationState {
  turns: Turn[];
  streaming: boolean;
  activity: Activity | null;
  startedAt: number | null;
  thread: string | null;
  turn: string | null;
  loading: boolean;
  /** How full the context was after the last model call, once known. */
  context: ContextUse | null;
}

/** How opening a thread went: open, archived, no longer there, or not readable. */
export type OpenResult = "open" | "archived" | "gone" | "error";

/** What a view adds to the message it sends. */
interface AskOptions {
  noteId?: string | undefined;
  provider: string | null;
  model: string | null;
  /** The message starts a thread of the agent, when the conversation has none yet. */
  startThread: boolean;
  /** The files attached to the message. */
  attachments?: Attachment[];
}

/**
 * The conversations of threads, by thread id: one per thread, shared by every view in the tab
 * that shows it, and kept while a view holds it or its turn runs on (agents G18).
 */
const conversations = new Map<string, Conversation>();

type Conversation = ReturnType<typeof createConversation>;

/**
 * One conversation with agent `agent`: what it shows, the transcript it sends, the turn it
 * follows, with the one event subscription a thread has, and the ways to act on it. `id` is its
 * thread, or null for one whose first message has not started a thread yet. Views hold it with
 * `hold` and let go with `release`; once nothing holds it and its turn has ended, it is gone.
 */
function createConversation(agent: string, id: string | null) {
  return createRoot((dispose) => {
    const [chat, setChat] = createStore<ConversationState>({
      turns: [],
      streaming: false,
      activity: null,
      startedAt: null,
      thread: id,
      turn: null,
      loading: false,
      context: null,
    });
  /** The views holding it, and what each wants told when it changes. */
  let holders = 0;
  const listeners = new Set<() => void>();
  /** Not read from the server yet. */
  let fresh = id !== null;

  function notify() {
    for (const l of [...listeners]) l();
  }
  /** The transcript so far, sent with each turn. Append-only. */
  let transcript: ChatMessage[] = [];
  /** Reads the followed turn's events; aborting it only stops reading (agents G17). */
  let controller: AbortController | null = null;
  /** Set by Stop, so the turn's end says it was stopped. */
  let stopping = false;
  /** A re-read turn's proposals as the thread holds them, by id, so their cards keep their state. */
  let replay = new Map<string, StoredProposal>();
  /** Each conversation's turns write only while it is the current one. */
  let generation = 0;

  /** Test-only: the transcript the next turn sends. */
  function transcriptNow(): readonly ChatMessage[] {
    return transcript;
  }

  /**
   * What the user did with proposals since the last message (ai D29), one line each, by proposal
   * id in the order they happened: a proposal acted on twice, failed then applied, keeps its last.
   * The next message sends them ahead of the question, once.
   */
  const outcomes = new Map<string, string>();

  function record(id: string, line: string) {
    outcomes.delete(id);
    outcomes.set(id, line);
  }

  /** Test-only: the outcome lines the next message sends. */
  function outcomesNow(): string[] {
    return [...outcomes.values()];
  }

  /**
   * Stop following the turn in flight. One on a thread runs on (agents G17); one without a
   * thread, which nothing keeps, is stopped.
   */
  function letGo() {
    generation++;
    if (chat.turn && !chat.thread) void api.aiTurnStop(chat.turn).catch(() => undefined);
    controller?.abort();
    controller = null;
  }

  /** Stop the turn in flight: on the server, the only way to end it early (agents G17). */
  function stop() {
    if (!chat.streaming) return;
    stopping = true;
    if (chat.turn) void api.aiTurnStop(chat.turn).catch(() => undefined);
    controller?.abort();
  }

  function lastTurn(): number {
    return chat.turns.length - 1;
  }

  function push(part: Part) {
    setChat("turns", lastTurn(), "parts", (parts) => [...parts, part]);
  }

  function appendMessages(data: { messages?: unknown }) {
    if (Array.isArray(data.messages)) transcript = [...transcript, ...(data.messages as ChatMessage[])];
  }

  function onEvent(e: StreamEvent) {
    const d = (e.data ?? {}) as Record<string, unknown>;
    const next = activityAfter(e, d);
    if (next !== undefined) setChat("activity", next);
    switch (e.event) {
      case "text": {
        const delta = String(d.delta ?? "");
        const i = lastTurn();
        const parts = chat.turns[i]?.parts ?? [];
        const last = parts[parts.length - 1];
        if (last?.kind === "text") setChat("turns", i, "parts", parts.length - 1, produce((p) => void ((p as { text: string }).text += delta)));
        else push({ kind: "text", text: delta });
        break;
      }
      case "tool":
        push({ kind: "tool", id: String(d.id), name: String(d.name), input: (d.input ?? null) as Json, done: false, isError: false, notes: [] });
        break;
      case "tool_result": {
        const i = lastTurn();
        const at = chat.turns[i]?.parts.findIndex((p) => p.kind === "tool" && p.id === d.id) ?? -1;
        if (at < 0) break;
        setChat(
          "turns",
          i,
          "parts",
          at,
          produce((p) => {
            if (p.kind === "tool") fillStep(p, d.summary, d.is_error === true);
          }),
        );
        break;
      }
      case "proposal":
      case "type_proposal":
      case "taxonomy_proposal":
      case "agent_proposal":
      case "agent_memory_proposal":
      case "skill_proposal": {
        // A re-read turn's proposal, applied or discarded meanwhile, shows as it stands.
        const stored = replay.get(String(d.id));
        push((stored && storedPart(stored)) || eventPart(e.event, d)!);
        break;
      }
      case "applied": {
        push(eventPart(e.event, d)!);
        // A re-read turn's change was settled when it was made.
        if (replay.has(String(d.id))) break;
        if (d.op === "download") break;
        const applied = d as unknown as Applied;
        // Open views of the note or type take the change, as after a write of this client's own.
        if (applied.op.startsWith("agent_memory_")) memoriesChanged();
        // A skill change shows on the Skills page, the sidebar's count and any open skill screen (skills S9).
        if (applied.op.startsWith("skill_")) {
          void skillsChanged().catch(() => undefined);
          break;
        }
        void settle(
          applied.op === "type_change" || applied.op === "type_create" || applied.op === "type_delete"
            ? "type"
            : applied.op.startsWith("taxonomy_")
              ? "taxonomy"
              : applied.op.startsWith("agent_") && !applied.op.startsWith("agent_memory_")
                ? "agent"
                : "note",
        );
        break;
      }
      case "refusal": {
        appendMessages(d);
        const s = (k: string) => (typeof d[k] === "string" ? (d[k] as string) : null);
        push({
          kind: "notice",
          notice: "refusal",
          category: s("category"),
          text: refusalText(s("provider"), s("model"), s("category"), s("explanation")),
        });
        break;
      }
      case "error":
        appendMessages(d);
        push({ kind: "notice", notice: "error", text: String(d.message ?? "The request failed.") });
        break;
      case "done":
        appendMessages(d);
        for (const n of noticesOf(d)) push(n);
        break;
      case "context": {
        if (typeof d.used === "number" && typeof d.window === "number") setChat("context", { used: d.used, window: d.window });
        break;
      }
      case "compacted": {
        push({ kind: "compaction", summary: String(d.summary ?? ""), reason: String(d.reason ?? "auto") });
        // A `/compact` turn's notice says it is done.
        if (d.reason === "command") {
          let at = chat.turns.length - 1;
          while (at >= 0 && chat.turns[at]!.role !== "user") at--;
          if (at >= 0) setChat("turns", at, "notice", COMPACTED);
        }
        break;
      }
      case "compact_failed":
        push({ kind: "notice", notice: "error", text: `The conversation was not compacted: ${String(d.message ?? "")}` });
        break;
      case "skills_updated": {
        // The server's user message of a `/update-skills` turn (skills S6), which a conversation
        // with no thread sends back as its own.
        const message = d.message as ChatMessage | undefined;
        if (message) transcript = [...transcript, message];
        const notice = message?.content.find((c) => c.type === "notice" && c.kind === SKILLS_UPDATED);
        const text = notice?.type === "notice" ? notice.text : `Skills updated (${Number(d.count ?? 0)})`;
        let at = chat.turns.length - 1;
        while (at >= 0 && chat.turns[at]!.role !== "user") at--;
        if (at >= 0) setChat("turns", at, "notice", text);
        break;
      }
    }
  }

  /**
   * Start the thread the next turn runs on, and keep the conversation as that thread's. Null when
   * the server has no threads, so the conversation stays in this tab; an error the user should
   * see throws.
   */
  async function startThread(mine: number): Promise<string | null> {
    try {
      const t = await api.agentThreadStart(agent);
      if (mine !== generation) return t.id;
      setChat("thread", t.id);
      if (!conversations.has(t.id)) conversations.set(t.id, self);
      notify();
      return t.id;
    } catch (e) {
      // A home server from before threads, or none reachable to ask: keep it here.
      if (!(e instanceof ApiError) || e.status === 404 || e.status === 405) return null;
      throw e;
    }
  }

  /**
   * Whether the thread can still carry the panel's next turn: false once it was archived or
   * deleted elsewhere (agents G15). A thread that cannot be read for another reason is kept, and
   * the turn says what went wrong.
   */
  async function stillOpen(): Promise<boolean> {
    if (!chat.thread) return true;
    try {
      return (await api.agentThread(agent, chat.thread)).archived !== true;
    } catch (e) {
      return !(e instanceof ApiError && e.status === 404);
    }
  }

  /** Ask `text`, with what the view adds: the note open beside it, the model, a new thread. */
  async function ask(text: string, o: AskOptions) {
    const files = o.attachments ?? [];
    const question = text.trim();
    if ((!question && files.length === 0) || chat.streaming) return;
    const mine = ++generation;
    // `/update-skills` alone takes a fresh list of skills and sends no message of the user's
    // (skills S6); any other `/` text is sent as typed.
    const updateSkills = text.trim() === UPDATE_SKILLS;
    // `/compact` alone summarizes the thread for the model now; a conversation with no thread
    // has nothing kept to compact.
    const compact = text.trim() === COMPACT;
    if (compact && !chat.thread) {
      setChat(
        produce((c) => {
          c.turns.push({ role: "user", text: "", notice: COMPACT, parts: [] });
          c.turns.push({ role: "assistant", parts: [{ kind: "notice", notice: "error", text: "There is nothing to compact yet." }] });
        }),
      );
      return;
    }
    const command = updateSkills || compact;
    // The proposals acted on since the last message, shown ahead of the question (ai D29).
    const lines = [...outcomes.values()];
    const kept = [...outcomes.entries()];
    outcomes.clear();
    replay = new Map();
    stopping = false;
    setChat(
      produce((c) => {
        const asked = updateSkills ? { text: "", notice: UPDATING_SKILLS } : compact ? { text: "", notice: COMPACTING } : { text: question };
        c.turns.push({
          role: "user",
          ...asked,
          parts: [],
          ...(lines.length && !command ? { outcomes: lines } : {}),
          ...(files.length && !command ? { attachments: [...files] } : {}),
        });
        c.turns.push({ role: "assistant", parts: [] });
        c.streaming = true;
        c.activity = { step: "thinking" };
        c.startedAt = Date.now();
      }),
    );
    const ctl = new AbortController();
    controller = ctl;
    try {
      const thread = o.startThread && !chat.thread ? await startThread(mine) : chat.thread;
      if (mine !== generation) return;
      // On a thread the server keeps the transcript and adds the outcome lines itself; here they
      // go first, as plain text every provider reads (ai D29), and only once.
      const content: Content[] = question ? [{ type: "text", text: question }] : [];
      if (files.length) content.unshift({ type: "text", text: attachmentText(files) });
      if (lines.length && !thread) content.unshift({ type: "text", text: lines.join("\n") });
      const user: ChatMessage = { role: "user", content };
      // A `/update-skills` turn's message is the server's: the outcome lines wait for the next
      // message, except on a thread, where the server adds them to its message.
      if (command && !thread) for (const [id, line] of kept) record(id, line);
      if (!command) {
        // Where the question sits in the transcript, for Edit and Fork.
        let asked = chat.turns.length - 1;
        while (asked >= 0 && chat.turns[asked]!.role !== "user") asked--;
        if (asked >= 0) setChat("turns", asked, "at", transcript.length);
        transcript = [...transcript, user];
      }
      const named = thread || agent !== ASSISTANT;
      const body = {
        ...(o.provider ? { provider: o.provider } : {}),
        ...(o.model ? { model: o.model } : {}),
        messages: command ? (thread ? [] : transcript) : thread ? [user] : transcript,
        ...(o.noteId ? { note_id: o.noteId } : {}),
        ...(named ? { agent } : {}),
        ...(thread ? { thread } : {}),
        ...(updateSkills ? { update_skills: true } : {}),
        ...(compact ? { compact: true } : {}),
      };
      const started = await api.aiTurnStart(body);
      if (mine !== generation || ctl.signal.aborted) {
        // Let go before the turn started: one without a thread is stopped; Stop stops any.
        if (!thread || ctl.signal.aborted) void api.aiTurnStop(started.turn).catch(() => undefined);
        if (mine !== generation) return;
        ctl.signal.throwIfAborted();
      }
      setChat({ turn: started.turn, ...(started.started_ms ? { startedAt: started.started_ms } : {}) });
      changed();
      if ((await follow(started.turn, mine, ctl)) === "gone" && thread) await reload(thread, mine);
    } catch (e) {
      ended(e, mine, ctl);
    } finally {
      settled(mine);
    }
  }

  /** The thread lists and the sidebar read again: a turn started or ended (agents G17). */
  function changed() {
    notify();
    bumpTurns(chat.thread ? { agent, thread: chat.thread, turn: chat.turn, running: chat.streaming } : null);
    void refreshAgents();
  }

  /** Show why the followed turn ended early, unless the conversation let it go. */
  function ended(e: unknown, mine: number, ctl: AbortController) {
    if (mine !== generation) return;
    if (ctl.signal.aborted || stopping) push({ kind: "notice", notice: "stopped", text: "Stopped." });
    else if (e instanceof ApiError && e.body.error === "thread_busy")
      push({ kind: "notice", notice: "busy", text: "Still answering: this thread's last question is still being answered. Wait for it, or open the thread to follow it." });
    else push({ kind: "notice", notice: "error", text: e instanceof Error ? e.message : String(e) });
  }

  /**
   * The followed turn is over, or this conversation stopped following it. A thread nobody shows
   * now has a new reply, and a conversation nothing holds any more is gone.
   */
  function settled(mine: number) {
    if (mine !== generation) return;
    controller = null;
    stopping = false;
    const followed = chat.turn !== null;
    setChat({ streaming: false, activity: null, startedAt: null, turn: null });
    changed();
    if (followed && chat.thread) turnEnded(chat.thread);
    drop();
  }

  /**
   * Read turn `turn`'s events into the conversation until the turn ends, reading again from the
   * last event seen when the stream breaks off. "gone" when the server no longer keeps the
   * turn's events, so the thread note is the one record of it.
   */
  async function follow(turn: string, mine: number, ctl: AbortController): Promise<"ended" | "gone"> {
    let seen = 0;
    let over = false;
    const each = (e: StreamEvent) => {
      if (mine !== generation) return;
      seen++;
      if (TERMINAL.has(e.event)) over = true;
      onEvent(e);
    };
    const app = await appInfo().catch(() => null);
    for (let tries = 0; ; tries++) {
      try {
        if (app) await pollTurn(turn, seen, each, ctl.signal);
        else await streamTurn(turn, seen, each, ctl.signal);
      } catch (e) {
        if (ctl.signal.aborted) throw e;
        if (e instanceof ApiError && (e.status === 404 || e.status === 410)) return "gone";
        if (e instanceof ApiError || tries >= REATTACH_TRIES) throw e;
      }
      // A stream that ended without the turn's end broke off, or the turn was stopped.
      if (over || mine !== generation) return "ended";
      ctl.signal.throwIfAborted();
      if (tries >= REATTACH_TRIES) return "ended";
      await sleep(500 * (tries + 1));
      ctl.signal.throwIfAborted();
    }
  }

  /** Show thread `id` as the server keeps it. */
  async function reload(id: string, mine: number) {
    const t = await api.agentThread(agent, id);
    if (mine !== generation) return;
    transcript = (t.messages ?? []) as ChatMessage[];
    setChat({ turns: turnsFromThread(t), context: t.context ?? null });
  }

  /**
   * Follow the turn running on thread `t` (agents G17): the thread as it stood before the turn,
   * then the turn's events from the first, so what it said while nobody watched shows again. A
   * trigger's run has no turn to read: the thread is read again until the run ends.
   */
  async function attach(t: AgentThread, mine: number) {
    const messages = (t.messages ?? []) as ChatMessage[];
    const before = t.turn ? messages.slice(0, turnStart(messages)) : messages;
    transcript = before;
    replay = new Map((t.proposals ?? []).map((p) => [String(p.data.id), p]));
    setChat({
      turns: [...turnsFromThread({ ...t, messages: before }), ...(t.turn ? [{ role: "assistant" as const, parts: [] }] : [])],
      loading: false,
      streaming: true,
      activity: { step: "thinking" },
      startedAt: t.started_ms ?? Date.now(),
      turn: t.turn ?? null,
    });
    const ctl = new AbortController();
    controller = ctl;
    try {
      if (t.turn) {
        if ((await follow(t.turn, mine, ctl)) === "gone") await reload(t.id, mine);
        return;
      }
      for (;;) {
        await sleep(WATCH_MS);
        if (mine !== generation) return;
        ctl.signal.throwIfAborted();
        const now = await api.agentThread(agent, t.id);
        if (mine !== generation) return;
        if (!now.running) {
          transcript = (now.messages ?? []) as ChatMessage[];
          setChat({ turns: turnsFromThread(now) });
          return;
        }
      }
    } catch (e) {
      ended(e, mine, ctl);
    } finally {
      settled(mine);
    }
  }

  /**
   * Read the thread: its turns as they were, its pending proposals open again, and a turn running
   * on it followed (agents G17).
   */
  async function load(): Promise<OpenResult> {
    fresh = false;
    const id = chat.thread;
    if (!id) return "open";
    const mine = generation;
    setChat("loading", true);
    try {
      const t = await api.agentThread(agent, id);
      if (mine !== generation) return "open";
      const result: OpenResult = t.archived === true ? "archived" : "open";
      if (t.running) {
        void attach(t, mine);
        return result;
      }
      transcript = (t.messages ?? []) as ChatMessage[];
      setChat({ turns: turnsFromThread(t), loading: false, context: t.context ?? null });
      return result;
    } catch (e) {
      if (mine !== generation) return "error";
      setChat({
        loading: false,
        turns: [{ role: "assistant", parts: [{ kind: "notice", notice: "error", text: e instanceof Error ? e.message : String(e) }] }],
      });
      return e instanceof ApiError && e.status === 404 ? "gone" : "error";
    }
  }

  /** Tell the thread the user discarded proposal `id` (agents G8). */
  function discarded(proposal: { id: string; thread?: unknown }) {
    const thread = typeof proposal.thread === "string" ? proposal.thread : null;
    if (!thread) return;
    void api
      .aiDiscard(proposal.id, thread)
      .then(() => notify())
      .catch(() => undefined);
  }

  function setProposal(turn: number, part: number, patch: { state: ProposalState; error?: string; created?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "proposal") return;
        p.state = patch.state;
        p.error = patch.error;
        if (patch.created) p.created = patch.created;
      }),
    );
  }

  /**
   * Apply a note proposal with POST /api/ai/apply, on the home server, as the chat sent it (ai
   * D23): an update, an archive or unarchive, or a delete, which moves the note to the trash, at
   * the version it was proposed on, or a create. A note that changed since is a conflict and
   * nothing is written.
   */
  async function applyProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "proposal" || (p.state !== "open" && p.state !== "failed")) return;
    const proposal = p.proposal;
    setProposal(turn, part, { state: "applying" });
    try {
      const done = await api.aiApply(proposal);
      const created = proposal.op === "create" && "note_id" in done ? done.note_id : undefined;
      setProposal(turn, part, { state: "applied", ...(created ? { created } : {}) });
      record(proposal.id, `[Applied by the user: ${proposalDone(proposal, done)}]`);
      await settle("note");
      notify();
    } catch (e) {
      const stale = e instanceof ApiError && (e.isConflict || e.status === 404);
      const error = e instanceof ApiError && e.isConflict ? NOTE_CHANGED : stale ? NOTE_GONE : e instanceof Error ? e.message : String(e);
      setProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(proposal.id, `[Failed to apply: ${proposalWhat(proposal)}: ${error}]`);
    }
  }

  function setDownload(turn: number, part: number, patch: { state: ProposalState; error?: string; asset?: DownloadedAsset }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "download") return;
        p.state = patch.state;
        p.error = patch.error;
        if (patch.asset) p.asset = patch.asset;
      }),
    );
  }

  /**
   * Apply a download proposal with POST /api/ai/apply (ai D23, D27): the home server downloads the
   * file and stores it as an asset. In an app the asset reaches the device with a sync round.
   */
  async function applyDownload(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "download" || (p.state !== "open" && p.state !== "failed")) return;
    setDownload(turn, part, { state: "applying" });
    try {
      const done = await api.aiApply(p.proposal);
      const asset = "asset_id" in done ? done : undefined;
      setDownload(turn, part, { state: "applied", ...(asset ? { asset } : {}) });
      const stored = asset ? ` stored as asset ${asset.asset_id} (${asset.filename})` : "";
      record(p.proposal.id, `[Applied by the user: ${downloadWhat(p.proposal)}${stored}]`);
      await settle("note");
      notify();
    } catch (e) {
      const error = e instanceof Error ? e.message : String(e);
      setDownload(turn, part, { state: "failed", error });
      record(p.proposal.id, `[Failed to apply: ${downloadWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardDownload(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "download" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setDownload(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${downloadWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  /**
   * Apply all: every pending proposal of the turn, in order, stopping at the first one that does
   * not apply (ai D20). Its card shows why; the rest stay pending.
   */
  async function applyAll(turn: number) {
    const parts = chat.turns[turn]?.parts ?? [];
    const todo = parts.flatMap((p, i) => (pending(p) ? [i] : []));
    for (const i of todo) {
      const p = chat.turns[turn]?.parts[i];
      if (!p || !pending(p)) continue;
      if (p.kind === "proposal") await applyProposal(turn, i);
      else if (p.kind === "download") await applyDownload(turn, i);
      else if (p.kind === "taxonomy_proposal") await applyTaxonomyProposal(turn, i);
      else if (p.kind === "agent_proposal") await applyAgentProposal(turn, i);
      else if (p.kind === "memory_proposal") await applyMemoryProposal(turn, i);
      else if (p.kind === "skill_proposal") await applySkillProposal(turn, i);
      else await applyTypeProposal(turn, i);
      const after = chat.turns[turn]?.parts[i];
      if (!after || !("state" in after) || after.state !== "applied") return;
    }
  }

  function discardProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${proposalWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  function setTypeProposal(turn: number, part: number, patch: { state: TypeProposalState; error?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "type_proposal") return;
        p.state = patch.state;
        p.error = patch.error;
      }),
    );
  }

  /**
   * Apply a type proposal with POST /api/ai/apply, on the home server (ai D18, D23): a new type, or
   * a change planned and applied as one commit, when the type file is still the one proposed
   * against. It needs admin: from an app, the server answers that it is done in the web app.
   */
  async function applyTypeProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "type_proposal" || (p.state !== "open" && p.state !== "failed")) return;
    setTypeProposal(turn, part, { state: "applying" });
    try {
      await api.aiApply(p.proposal);
      setTypeProposal(turn, part, { state: "applied" });
      const done = p.proposal.op === "delete" ? `deleted type ${p.proposal.name}` : p.proposal.exists ? typeWhat(p.proposal) : `created type ${p.proposal.name}`;
      record(p.proposal.id, `[Applied by the user: ${done}]`);
      await settle("type");
      notify();
    } catch (e) {
      // A type delete the type's notes now refuse is a 409 too, with its own reason.
      const stale = e instanceof ApiError && e.status === 409 && e.body.error === "stale";
      const error = stale ? TYPE_CHANGED : e instanceof Error ? e.message : String(e);
      setTypeProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(p.proposal.id, `[Failed to apply: ${typeWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardTypeProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "type_proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setTypeProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${typeWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  function setTaxonomyProposal(turn: number, part: number, patch: { state: ProposalState; error?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "taxonomy_proposal") return;
        p.state = patch.state;
        p.error = patch.error;
      }),
    );
  }

  /**
   * Apply a taxonomy proposal with POST /api/ai/apply, on the home server (ai D31): the new file
   * with its renames, which move the notes holding the renamed terms, or the delete. The server
   * first re-reads the taxonomy and writes nothing when it changed since the proposal (a 409). It
   * needs admin: from an app, the server answers that it is done in the web app.
   */
  async function applyTaxonomyProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "taxonomy_proposal" || (p.state !== "open" && p.state !== "failed")) return;
    setTaxonomyProposal(turn, part, { state: "applying" });
    try {
      const done = await api.aiApply(p.proposal);
      setTaxonomyProposal(turn, part, { state: "applied" });
      record(p.proposal.id, `[Applied by the user: ${taxonomyDone(p.proposal, done)}]`);
      await settle("taxonomy");
      notify();
    } catch (e) {
      const stale = e instanceof ApiError && e.status === 409 && e.body.error === "stale";
      const error = stale ? TAXONOMY_CHANGED : e instanceof Error ? e.message : String(e);
      setTaxonomyProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(p.proposal.id, `[Failed to apply: ${taxonomyWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardTaxonomyProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "taxonomy_proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setTaxonomyProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${taxonomyWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  function setAgentProposal(turn: number, part: number, patch: { state: ProposalState; error?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "agent_proposal") return;
        p.state = patch.state;
        p.error = patch.error;
      }),
    );
  }

  /**
   * Apply an agent proposal with POST /api/ai/apply, on the home server (agents G20): the new
   * agent file, or the delete. The server re-reads the file and writes nothing when it changed
   * since the proposal (a 409), and checks the change again against the proposing agent. It
   * needs admin: from an app, the server answers that it is done in the web app. The sidebar's
   * agent list reads again after it.
   */
  async function applyAgentProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "agent_proposal" || (p.state !== "open" && p.state !== "failed")) return;
    setAgentProposal(turn, part, { state: "applying" });
    try {
      await api.aiApply(p.proposal);
      setAgentProposal(turn, part, { state: "applied" });
      record(p.proposal.id, `[Applied by the user: ${agentDone(p.proposal)}]`);
      await settle("agent");
      notify();
    } catch (e) {
      const stale = e instanceof ApiError && e.status === 409 && e.body.error === "stale";
      const error = stale ? AGENT_CHANGED : e instanceof Error ? e.message : String(e);
      setAgentProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(p.proposal.id, `[Failed to apply: ${agentWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardAgentProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "agent_proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setAgentProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${agentWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  function setMemoryProposal(turn: number, part: number, patch: { state: ProposalState; error?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "memory_proposal") return;
        p.state = patch.state;
        p.error = patch.error;
      }),
    );
  }

  /**
   * Apply a memory proposal with POST /api/ai/apply (agents G21): the new memory, the change or
   * the forgetting. The server writes nothing when the memory changed since the proposal (a
   * 409), and checks the change again against the proposing agent. Memories are notes, so it
   * needs only `write` and works from the apps; there it waits for the sync round.
   */
  async function applyMemoryProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "memory_proposal" || (p.state !== "open" && p.state !== "failed")) return;
    setMemoryProposal(turn, part, { state: "applying" });
    try {
      const done = await api.aiApply(p.proposal);
      setMemoryProposal(turn, part, { state: "applied" });
      record(p.proposal.id, `[Applied by the user: ${memoryDone(p.proposal, done)}]`);
      memoriesChanged();
      await settle("note");
      notify();
    } catch (e) {
      const stale = e instanceof ApiError && e.status === 409 && e.body.error === "stale";
      const error = stale ? MEMORY_CHANGED : e instanceof Error ? e.message : String(e);
      setMemoryProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(p.proposal.id, `[Failed to apply: ${memoryWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardMemoryProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "memory_proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setMemoryProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${memoryWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

  function setSkillProposal(turn: number, part: number, patch: { state: ProposalState; error?: string }) {
    setChat(
      "turns",
      turn,
      "parts",
      part,
      produce((p) => {
        if (p.kind !== "skill_proposal") return;
        p.state = patch.state;
        p.error = patch.error;
      }),
    );
  }

  /**
   * Apply a skill proposal with POST /api/ai/apply, on the home server (skills S9): the new
   * `SKILL.md`, the file, or the delete. The server re-reads what it replaces and writes nothing
   * when it changed since the proposal (a 409). It needs admin: from an app, the server answers
   * that it is done in the web app. The Skills page and the sidebar's count read again after it.
   */
  async function applySkillProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "skill_proposal" || (p.state !== "open" && p.state !== "failed")) return;
    setSkillProposal(turn, part, { state: "applying" });
    try {
      const done = await api.aiApply(p.proposal);
      setSkillProposal(turn, part, { state: "applied" });
      record(p.proposal.id, `[Applied by the user: ${skillDone(p.proposal, done)}]`);
      await skillsChanged().catch(() => undefined);
      notify();
    } catch (e) {
      const stale = e instanceof ApiError && e.status === 409 && e.body.error === "stale";
      const error = stale ? SKILL_CHANGED : e instanceof Error ? e.message : String(e);
      setSkillProposal(turn, part, { state: stale ? "stale" : "failed", error });
      record(p.proposal.id, `[Failed to apply: ${skillWhat(p.proposal)}: ${error}]`);
    }
  }

  function discardSkillProposal(turn: number, part: number) {
    const p = chat.turns[turn]?.parts[part];
    if (p?.kind !== "skill_proposal" || p.state === "applied" || p.state === "applying" || p.state === "discarded") return;
    setSkillProposal(turn, part, { state: "discarded" });
    record(p.proposal.id, `[Discarded by the user: ${skillWhat(p.proposal)}]`);
    discarded(p.proposal);
  }

    /** A view shows the conversation; `listener` hears of each change until it lets go. */
    function hold(listener?: () => void) {
      holders++;
      if (listener) listeners.add(listener);
    }

    /** A view no longer shows the conversation. */
    function release(listener?: () => void) {
      holders = Math.max(0, holders - 1);
      if (listener) listeners.delete(listener);
      drop();
    }

    /**
     * Once no view holds it: a conversation whose thread's turn runs follows it to its end, so a
     * view that comes back finds it as it stands; any other is gone. A turn without a thread,
     * which nothing keeps, is stopped (agents G17).
     */
    function drop(force = false) {
      if (!force && (holders > 0 || (chat.streaming && chat.thread))) return;
      letGo();
      listeners.clear();
      if (chat.thread && conversations.get(chat.thread) === self) conversations.delete(chat.thread);
      dispose();
    }

    /** What Edit and Fork give back to the message box: the question and its files. */
    function draftOf(turn: Turn) {
      return { text: turn.text ?? "", attachments: turn.attachments ?? [] };
    }

    /**
     * Edit question `t`: the conversation is cut back to before it, on the server for a thread,
     * and the question comes back for the box. Null while a turn runs or for no question.
     */
    async function editFrom(t: number): Promise<{ text: string; attachments: Attachment[] } | null> {
      const turn = chat.turns[t];
      if (!turn || turn.role !== "user" || turn.at === undefined || chat.streaming) return null;
      const mine = ++generation;
      if (chat.thread) {
        const cut = await api.agentThreadTruncate(agent, chat.thread, turn.at);
        if (mine !== generation) return null;
        transcript = (cut.messages ?? []) as ChatMessage[];
        setChat({ turns: turnsFromThread(cut), context: cut.context ?? null });
      } else {
        transcript = transcript.slice(0, turn.at);
        setChat({ turns: chat.turns.slice(0, t), context: null });
      }
      changed();
      return draftOf(turn);
    }

    /** Fork at question `t`: a new thread with what came before it; this one stays as it is. */
    async function forkFrom(t: number): Promise<{ thread: string; text: string; attachments: Attachment[] } | null> {
      const turn = chat.turns[t];
      if (!turn || turn.role !== "user" || turn.at === undefined || !chat.thread) return null;
      const made = await api.agentThreadFork(agent, chat.thread, turn.at);
      notify();
      void refreshAgents();
      return { thread: made.id, ...draftOf(turn) };
    }

    const self = {
      agent,
      chat: chat as Readonly<ConversationState>,
      editFrom,
      forkFrom,
      hold,
      release,
      drop,
      load,
      isFresh: () => fresh,
      ask,
      stop,
      stillOpen,
      transcriptNow,
      outcomesNow,
      applyProposal,
      discardProposal,
      applyDownload,
      discardDownload,
      applyAll,
      applyTypeProposal,
      discardTypeProposal,
      applyTaxonomyProposal,
      discardTaxonomyProposal,
      applyAgentProposal,
      discardAgentProposal,
      applyMemoryProposal,
      discardMemoryProposal,
      applySkillProposal,
      discardSkillProposal,
    };
    return self;
  });
}

/** Hold thread `id`'s conversation, the one every view of it shares (agents G18). */
function holdThread(agent: string, id: string, listener: () => void): Conversation {
  let c = conversations.get(id);
  if (!c) {
    c = createConversation(agent, id);
    conversations.set(id, c);
  }
  c.hold(listener);
  return c;
}

/** Test-only: let every thread's conversation go. */
export function resetThreads() {
  for (const c of [...conversations.values()]) c.drop(true);
  conversations.clear();
}

/** Test-only: how many threads have a conversation now. */
export function threadsHeld(): number {
  return conversations.size;
}

/** Whether agent `name` names its own provider or model, which the chat box's choice overrides. */
export function ownsModel(name: string): boolean {
  const a = agents().find((x) => x.name === name);
  return !!a && (!!a.provider || !!a.model);
}

/** The conversation a view shows, as the views read it: the thread's, and the view's own choices. */
export interface ChatState extends ConversationState {
  /** The provider and model the next turn uses; null for Settings > AI's defaults. */
  provider: string | null;
  model: string | null;
  /** The agent whose conversation this is (agents G6). */
  agent: string;
}

/**
 * One view of conversations, such as the right sidebar's AI panel or an agent screen's chat:
 * the agent, provider and model it chose, and the conversation it shows, a new one or a thread's.
 * A thread's conversation is shared with every other view showing that thread (agents G18), so
 * a reply streams in both, and sending, Stop, Apply and Discard in either show in both; each view
 * chooses its threads by itself.
 */
export function createChatSession(options: ChatOptions = {}) {
  return createRoot((dispose) => {
    const [view, setView] = createStore({
      agent: options.agent ?? ASSISTANT,
      provider: null as string | null,
      model: null as string | null,
      /** The choice for an agent that names its own model, which overrides it; null keeps the agent's. */
      own: null as { agent: string; provider: string | null; model: string | null } | null,
    });
    /** Whether the view's agent names its own provider or model, so the choice is its override. */
    const owns = () => ownsModel(view.agent);
    const override = () => (view.own?.agent === view.agent ? view.own : null);
    /** While the panel checks that its thread can still take the next message. */
    const [checking, setChecking] = createSignal(false);
    const listener = () => options.onChange?.();
    const fresh = () =>
      untrack(() => {
        const c = createConversation(view.agent, null);
        c.hold(listener);
        return c;
      });
    const [current, setCurrent] = createSignal<Conversation>(fresh());

    /** Show conversation `next`, letting the one shown go. */
    function show(next: Conversation) {
      const was = untrack(current);
      setCurrent(() => next);
      if (was !== next) was.release(listener);
    }

    const chat: Readonly<ChatState> = {
      get turns() {
        return current().chat.turns;
      },
      get streaming() {
        return checking() || current().chat.streaming;
      },
      get activity() {
        return current().chat.activity;
      },
      get startedAt() {
        return current().chat.startedAt;
      },
      get context() {
        return current().chat.context;
      },
      get thread() {
        return current().chat.thread;
      },
      get turn() {
        return current().chat.turn;
      },
      get loading() {
        return current().chat.loading;
      },
      get provider() {
        return owns() ? (override()?.provider ?? null) : view.provider;
      },
      get model() {
        return owns() ? (override()?.model ?? null) : view.model;
      },
      get agent() {
        return view.agent;
      },
    };

    /**
     * Choose the provider and model of the next turns. For an agent that names its own, the choice
     * is kept apart, for that agent only, so it never leaks to another conversation; both null is
     * the agent's own again.
     */
    function setProvider(provider: string | null, model: string | null) {
      if (!owns()) setView({ provider, model });
      else setView("own", provider || model ? { agent: view.agent, provider, model } : null);
    }

    function setModel(model: string | null) {
      if (!owns()) setView("model", model);
      else setProvider(override()?.provider ?? null, model);
    }

    /** Start afresh. A turn in flight on a thread runs on, and its thread shows it when opened. */
    function newConversation() {
      setChecking(false);
      show(fresh());
    }

    /** Make this a conversation with agent `name`; another agent's conversation starts afresh. */
    function setAgent(name: string) {
      if (name === view.agent) return;
      setView({ agent: name, own: null });
      newConversation();
    }

    /** Show thread `id` of the agent, as every view of it shows it (agents G18). */
    async function openThread(id: string): Promise<OpenResult> {
      if (untrack(current).chat.thread === id) return "open";
      setChecking(false);
      const c = holdThread(view.agent, id, listener);
      show(c);
      return c.isFresh() ? c.load() : "open";
    }

    /** Ask `text`, beside the note `noteId` when one is open. */
    async function ask(text: string, noteId?: string, attachments: Attachment[] = []) {
      const question = text.trim();
      if ((!question && attachments.length === 0) || chat.streaming) return;
      // The panel's thread, archived or deleted elsewhere, gives way to a new one (agents G15). A
      // The agent screen keeps its thread: sending in an archived thread takes it out of the archive.
      const was = current();
      if (options.threads === "listed" && was.chat.thread) {
        setChecking(true);
        const open = await was.stillOpen();
        if (current() !== was || !checking()) return;
        setChecking(false);
        if (!open) newConversation();
      }
      const c = current();
      // The panel keeps threads only for an agent the server has listed, so a server from before
      // agents is never asked for one.
      const startThread = !!options.threads && (options.threads !== "listed" || agents().some((a) => a.name === view.agent));
      await c.ask(question, { noteId, provider: chat.provider, model: chat.model, startThread, attachments });
    }

    /** Stop the turn in flight: on the server, the only way to end it early (agents G17). */
    function stop() {
      setChecking(false);
      current().stop();
    }

    /** Let the conversation go. A turn in flight on a thread runs on (agents G17). */
    function close() {
      untrack(current).release(listener);
      dispose();
    }

    return {
      chat,
      ask,
      stop,
      close,
      newConversation,
      setProvider,
      setModel,
      setAgent,
      openThread,
      /** Edit question `turn`: cut the conversation back to before it, and give it back for the box. */
      editFrom: (turn: number) => current().editFrom(turn),
      /** Fork at question `turn` into a new thread, open it, and give the question back for the box. */
      forkFrom: async (turn: number) => {
        const made = await current().forkFrom(turn);
        if (!made) return null;
        await openThread(made.thread);
        return made;
      },
      /** Test-only: the transcript the next turn sends. */
      transcriptNow: () => current().transcriptNow(),
      /** Test-only: the outcome lines the next message sends. */
      outcomesNow: () => current().outcomesNow(),
      applyProposal: (turn: number, part: number) => current().applyProposal(turn, part),
      discardProposal: (turn: number, part: number) => current().discardProposal(turn, part),
      applyDownload: (turn: number, part: number) => current().applyDownload(turn, part),
      discardDownload: (turn: number, part: number) => current().discardDownload(turn, part),
      applyAll: (turn: number) => current().applyAll(turn),
      applyTypeProposal: (turn: number, part: number) => current().applyTypeProposal(turn, part),
      discardTypeProposal: (turn: number, part: number) => current().discardTypeProposal(turn, part),
      applyTaxonomyProposal: (turn: number, part: number) => current().applyTaxonomyProposal(turn, part),
      discardTaxonomyProposal: (turn: number, part: number) => current().discardTaxonomyProposal(turn, part),
      applyAgentProposal: (turn: number, part: number) => current().applyAgentProposal(turn, part),
      discardAgentProposal: (turn: number, part: number) => current().discardAgentProposal(turn, part),
      applyMemoryProposal: (turn: number, part: number) => current().applyMemoryProposal(turn, part),
      discardMemoryProposal: (turn: number, part: number) => current().discardMemoryProposal(turn, part),
      applySkillProposal: (turn: number, part: number) => current().applySkillProposal(turn, part),
      discardSkillProposal: (turn: number, part: number) => current().discardSkillProposal(turn, part),
    };
  });
}

export type ChatSession = ReturnType<typeof createChatSession>;

/** The right sidebar's AI panel's conversation: a thread of the agent it has chosen (agents G6). */
const panel = createChatSession({ threads: "listed" });

/** The panel's conversation itself, for the views that take a session. */
export const panelSession = panel;

export const chat = panel.chat;
export const ask = panel.ask;
export const stop = panel.stop;
export const newConversation = panel.newConversation;
export const setProvider = panel.setProvider;
export const setModel = panel.setModel;
export const setAgent = panel.setAgent;
export const openThread = panel.openThread;
/** Test-only: the transcript the next turn sends. */
export const transcriptNow = panel.transcriptNow;
/** Test-only: the outcome lines the next message sends. */
export const outcomesNow = panel.outcomesNow;
export const applyProposal = panel.applyProposal;
export const discardProposal = panel.discardProposal;
export const applyDownload = panel.applyDownload;
export const discardDownload = panel.discardDownload;
export const applyAll = panel.applyAll;
export const applyTypeProposal = panel.applyTypeProposal;
export const discardTypeProposal = panel.discardTypeProposal;
export const applyTaxonomyProposal = panel.applyTaxonomyProposal;
export const discardTaxonomyProposal = panel.discardTaxonomyProposal;
export const applyAgentProposal = panel.applyAgentProposal;
export const discardAgentProposal = panel.discardAgentProposal;
export const applyMemoryProposal = panel.applyMemoryProposal;
export const discardMemoryProposal = panel.discardMemoryProposal;
export const applySkillProposal = panel.applySkillProposal;
export const discardSkillProposal = panel.discardSkillProposal;

/** The conversation the cards below act on: the panel's, unless the agent screen provides its own. */
export const ChatContext = createContext<ChatSession>(panel);

export function useChatSession(): ChatSession {
  return useContext(ChatContext);
}
