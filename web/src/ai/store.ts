// The AI chat panel's conversation (ai D10, D14, D16): what the panel shows, and the neutral
// transcript the server takes back each turn. It lives in this tab's memory only; nothing is
// stored in the vault. A turn streams from POST /api/ai/chat, where the server runs the note
// tools; edits come back as proposals the user applies through the notes API, or, for the kinds
// Settings > AI lets go ahead without asking, as `applied` events for changes already made.

import { createRoot, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { type Json, type Note, type NoteDoc, ApiError, type StreamEvent, api, isArchived, streamEvents } from "../api";
import { noteWritten } from "../changes";
import { appInfo } from "../inApp";
import { reloadDefinitions } from "../session";

/** One piece of a transcript message, as the server's `ai::Content`. */
export type Content =
  | { type: "text"; text: string }
  | { type: "tool_call"; id: string; name: string; input: Json }
  | { type: "tool_result"; id: string; content: string; is_error?: boolean; parts?: ResultPart[] }
  | { type: "notice"; kind: string; text: string };

/** A tool result's part beyond its text (ai D19): an image by asset id, never its bytes. */
export type ResultPart = { type: "text"; text: string } | { type: "image"; asset_id: string; media_type: string };

/** The asset a `read_asset` step read, from its `tool_result` summary. */
export interface AssetRef {
  id: string;
  filename: string;
  media_type: string;
  size: number;
  /** `image`, `pdf`, `text` or `metadata`: what the model got. */
  kind: string;
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
  /** The type file as stored when proposed; null for a new type. */
  before: string | null;
  after: string;
  /** Warnings the check found, such as an attribute this version ignores. */
  issues: { at?: string; message: string; severity?: string }[];
}

/** As a note proposal's: `stale` when the type changed since the proposal. */
export type TypeProposalState = ProposalState;

/** What the card says when the type changed after the proposal (ai D18). */
export const TYPE_CHANGED = "This type changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/**
 * A change the AI made itself, its confirmation turned off in Settings > AI (ai D22): a note's,
 * with its new version (for a delete, the version that went to the trash), or a type's.
 */
export type Applied =
  | { id: string; op: NoteOp; note_id: string; title: Json; version: string }
  | { id: string; op: "type_change" | "type_create"; type_name: string };

export type Part =
  | { kind: "text"; text: string }
  | { kind: "applied"; applied: Applied }
  | { kind: "tool"; id: string; name: string; input: Json; done: boolean; isError: boolean; notes: NoteRef[]; message?: string; asset?: AssetRef }
  | { kind: "proposal"; proposal: Proposal; state: ProposalState; error?: string; created?: string }
  | { kind: "type_proposal"; proposal: TypeProposal; state: TypeProposalState; error?: string }
  | { kind: "notice"; notice: string; text: string; category?: string | null };

export interface Turn {
  role: "user" | "assistant";
  /** A user turn's question. */
  text?: string;
  /** An assistant turn's answer, tool steps, proposals and notices, in the order they came. */
  parts: Part[];
}

export interface ChatState {
  turns: Turn[];
  streaming: boolean;
  /** The provider and model the next turn uses; null for Settings > AI's defaults. */
  provider: string | null;
  model: string | null;
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

const state = createRoot(() => {
  const [chat, setChat] = createStore<ChatState>({ turns: [], streaming: false, provider: null, model: null });
  const [web, setWeb] = createSignal(false);
  return { chat, setChat, web, setWeb };
});

export const chat = state.chat;

let asked = false;

/**
 * Reactive: whether the page is the web app rather than the desktop or Android app, whose
 * embedded server has no AI credentials (ai D16). False until the answer is known.
 */
export function webApp(): boolean {
  if (!asked) {
    asked = true;
    void appInfo().then((a) => state.setWeb(a === null));
  }
  return state.web();
}

/** The transcript so far, sent with each turn. Append-only. */
let transcript: ChatMessage[] = [];
let controller: AbortController | null = null;
/** Each conversation's turns write only while it is the current one. */
let generation = 0;

/** Test-only: the transcript the next turn sends. */
export function transcriptNow(): readonly ChatMessage[] {
  return transcript;
}

export function setProvider(provider: string | null, model: string | null) {
  state.setChat({ provider, model });
}

export function setModel(model: string | null) {
  state.setChat("model", model);
}

/** Forget the conversation and stop a turn in flight. */
export function newConversation() {
  generation++;
  controller?.abort();
  controller = null;
  transcript = [];
  state.setChat({ turns: [], streaming: false });
}

/** Stop the turn in flight: the request is aborted, and the server stops when it sees that. */
export function stop() {
  controller?.abort();
}

function lastTurn(): number {
  return state.chat.turns.length - 1;
}

function push(part: Part) {
  state.setChat("turns", lastTurn(), "parts", (parts) => [...parts, part]);
}

function appendMessages(data: { messages?: unknown }) {
  if (Array.isArray(data.messages)) transcript = [...transcript, ...(data.messages as ChatMessage[])];
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

function onEvent(e: StreamEvent) {
  const d = (e.data ?? {}) as Record<string, unknown>;
  switch (e.event) {
    case "text": {
      const delta = String(d.delta ?? "");
      const i = lastTurn();
      const parts = state.chat.turns[i]?.parts ?? [];
      const last = parts[parts.length - 1];
      if (last?.kind === "text") state.setChat("turns", i, "parts", parts.length - 1, produce((p) => void ((p as { text: string }).text += delta)));
      else push({ kind: "text", text: delta });
      break;
    }
    case "tool":
      push({ kind: "tool", id: String(d.id), name: String(d.name), input: (d.input ?? null) as Json, done: false, isError: false, notes: [] });
      break;
    case "tool_result": {
      const i = lastTurn();
      const at = state.chat.turns[i]?.parts.findIndex((p) => p.kind === "tool" && p.id === d.id) ?? -1;
      if (at < 0) break;
      const summary = (d.summary ?? {}) as { notes?: NoteRef[]; message?: string; asset?: AssetRef };
      state.setChat(
        "turns",
        i,
        "parts",
        at,
        produce((p) => {
          if (p.kind !== "tool") return;
          p.done = true;
          p.isError = d.is_error === true;
          p.notes = (summary.notes ?? []).filter((n) => n && typeof n.id === "string").map((n) => ({ id: n.id, title: String(n.title ?? n.id) }));
          if (summary.message) p.message = summary.message;
          if (summary.asset && typeof summary.asset.id === "string") p.asset = summary.asset;
        }),
      );
      break;
    }
    case "proposal":
      push({ kind: "proposal", proposal: { ...d, op: typeof d.op === "string" ? d.op : "update" } as unknown as Proposal, state: "open" });
      break;
    case "type_proposal":
      push({ kind: "type_proposal", proposal: { ...(d as unknown as TypeProposal), issues: Array.isArray(d.issues) ? (d.issues as TypeProposal["issues"]) : [] }, state: "open" });
      break;
    case "applied": {
      const applied = d as unknown as Applied;
      push({ kind: "applied", applied });
      // Open views of the note or type take the change, as after a write of this client's own.
      if (applied.op === "type_change" || applied.op === "type_create") void reloadDefinitions().catch(() => undefined);
      else noteWritten();
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
  }
}

/** Ask `text`, beside the note `noteId` when one is open. */
export async function ask(text: string, noteId?: string) {
  const question = text.trim();
  if (!question || state.chat.streaming) return;
  const mine = ++generation;
  const user: ChatMessage = { role: "user", content: [{ type: "text", text: question }] };
  transcript = [...transcript, user];
  state.setChat(
    produce((c) => {
      c.turns.push({ role: "user", text: question, parts: [] });
      c.turns.push({ role: "assistant", parts: [] });
      c.streaming = true;
    }),
  );
  const ctl = new AbortController();
  controller = ctl;
  const body = {
    ...(state.chat.provider ? { provider: state.chat.provider } : {}),
    ...(state.chat.model ? { model: state.chat.model } : {}),
    messages: transcript,
    ...(noteId ? { note_id: noteId } : {}),
  };
  try {
    await streamEvents("/api/ai/chat", body, (e) => mine === generation && onEvent(e), ctl.signal);
  } catch (e) {
    if (mine !== generation) return;
    if (ctl.signal.aborted) push({ kind: "notice", notice: "stopped", text: "Stopped." });
    else push({ kind: "notice", notice: "error", text: e instanceof Error ? e.message : String(e) });
  } finally {
    if (mine === generation) {
      controller = null;
      state.setChat("streaming", false);
    }
  }
}

function setProposal(turn: number, part: number, patch: { state: ProposalState; error?: string; created?: string }) {
  state.setChat(
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

/** What a card says when the note changed after the proposal (ai D20). */
export const NOTE_CHANGED = "The note changed after this was proposed, so it was not applied. Ask again for a fresh proposal.";

/** Thrown inside an apply when the note is no longer as proposed: the card shows it, stale. */
class Stale extends Error {}

/** The note `id` as it is now, read again before an archive or delete is applied. */
async function currentNote(id: string): Promise<Note> {
  try {
    return await api.note(id);
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) throw new Stale("This note is no longer there, so nothing was applied.");
    throw e;
  }
}

/**
 * Apply a note proposal through the notes API (ai D14, D20): an update at the version it was
 * based on, a create, an archive or unarchive, or a delete, which moves the note to the trash.
 * An archive or delete reads the note again first, and does nothing to a note whose archive
 * state already changed or, for a delete, whose version moved past the proposal's.
 */
export async function applyProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "proposal" || (p.state !== "open" && p.state !== "failed")) return;
  const proposal = p.proposal;
  setProposal(turn, part, { state: "applying" });
  try {
    switch (proposal.op) {
      case "update":
        await api.updateNote(proposal.note_id, proposal.after, proposal.base_version);
        break;
      case "create": {
        const n = await api.createNote(proposal.type, proposal.after);
        setProposal(turn, part, { state: "applied", created: n.id });
        return;
      }
      case "archive":
      case "unarchive": {
        const n = await currentNote(proposal.note_id);
        const want = proposal.op === "archive";
        if (isArchived(n.note.fields) === want) {
          throw new Stale(want ? "This note is already archived, so nothing was applied." : "This note is no longer archived, so nothing was applied.");
        }
        await (want ? api.archiveNote(proposal.note_id, n.version) : api.unarchiveNote(proposal.note_id, n.version));
        break;
      }
      case "delete": {
        const n = await currentNote(proposal.note_id);
        if (n.version !== proposal.base_version) throw new Stale(NOTE_CHANGED);
        await api.deleteNote(proposal.note_id, proposal.base_version);
        break;
      }
    }
    setProposal(turn, part, { state: "applied" });
  } catch (e) {
    if (e instanceof Stale) setProposal(turn, part, { state: "stale", error: e.message });
    else if (e instanceof ApiError && e.isConflict) setProposal(turn, part, { state: proposal.op === "update" ? "failed" : "stale", error: NOTE_CHANGED });
    else setProposal(turn, part, { state: "failed", error: e instanceof Error ? e.message : String(e) });
  }
}

/** Whether a part is a proposal still waiting for the user: open, or failed and retryable. */
export function pending(p: Part): boolean {
  return (p.kind === "proposal" || p.kind === "type_proposal") && (p.state === "open" || p.state === "failed");
}

/**
 * Apply all: every pending proposal of the turn, in order, stopping at the first one that does
 * not apply (ai D20). Its card shows why; the rest stay pending.
 */
export async function applyAll(turn: number) {
  const parts = state.chat.turns[turn]?.parts ?? [];
  const todo = parts.flatMap((p, i) => (pending(p) ? [i] : []));
  for (const i of todo) {
    const p = state.chat.turns[turn]?.parts[i];
    if (!p || !pending(p)) continue;
    if (p.kind === "proposal") await applyProposal(turn, i);
    else await applyTypeProposal(turn, i);
    const after = state.chat.turns[turn]?.parts[i];
    if (!after || (after.kind !== "proposal" && after.kind !== "type_proposal") || after.state !== "applied") return;
  }
}

export function discardProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind === "proposal" && p.state !== "applied" && p.state !== "applying") setProposal(turn, part, { state: "discarded" });
}

function setTypeProposal(turn: number, part: number, patch: { state: TypeProposalState; error?: string }) {
  state.setChat(
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

/** The type file `name` as stored now, or null when there is no such type. */
async function currentType(name: string): Promise<string | null> {
  try {
    return (await api.readType(name)).wcl;
  } catch (e) {
    if (e instanceof ApiError && (e.status === 404 || e.body.error === "unknown_type")) return null;
    throw e;
  }
}

/**
 * Apply a type proposal through the type routes, as the type editor does: a new type with
 * `PUT /api/types/{name}`, a change planned and then applied with the plan's token. The type is
 * read again first; one that changed since the proposal is not applied (ai D18).
 */
export async function applyTypeProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "type_proposal" || (p.state !== "open" && p.state !== "failed")) return;
  const { name, before, after } = p.proposal;
  setTypeProposal(turn, part, { state: "applying" });
  try {
    if ((await currentType(name)) !== (before ?? null)) {
      setTypeProposal(turn, part, { state: "stale", error: TYPE_CHANGED });
      return;
    }
    if (before === null || before === undefined) {
      await api.putType(name, after);
    } else {
      const plan = await api.changeType(name, after, {});
      await api.changeType(name, after, {}, plan.plan);
    }
    setTypeProposal(turn, part, { state: "applied" });
    await reloadDefinitions().catch(() => undefined);
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) setTypeProposal(turn, part, { state: "stale", error: TYPE_CHANGED });
    else setTypeProposal(turn, part, { state: "failed", error: e instanceof Error ? e.message : String(e) });
  }
}

export function discardTypeProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind === "type_proposal" && p.state !== "applied" && p.state !== "applying") setTypeProposal(turn, part, { state: "discarded" });
}
