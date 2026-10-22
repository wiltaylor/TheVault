// The AI chat panel's conversation (ai D10, D14, D16): what the panel shows, and the neutral
// transcript the server takes back each turn. It lives in this tab's memory only; nothing is
// stored in the vault. A turn streams from POST /api/ai/chat, where the server runs the note
// tools; edits come back as proposals the user applies through the notes API.

import { createRoot, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { type Json, type NoteDoc, ApiError, type StreamEvent, api, streamEvents } from "../api";
import { appInfo } from "../inApp";

/** One piece of a transcript message, as the server's `ai::Content`. */
export type Content =
  | { type: "text"; text: string }
  | { type: "tool_call"; id: string; name: string; input: Json }
  | { type: "tool_result"; id: string; content: string; is_error?: boolean }
  | { type: "notice"; kind: string; text: string };

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

/** An edit the model proposed, checked by the server but not written (ai D14). */
export interface Proposal {
  id: string;
  note_id: string;
  title: Json;
  base_version: string;
  before: NoteDoc;
  after: NoteDoc;
  ops: ProposalOp[];
}

export type ProposalState = "open" | "applying" | "applied" | "discarded" | "failed";

export type Part =
  | { kind: "text"; text: string }
  | { kind: "tool"; id: string; name: string; input: Json; done: boolean; isError: boolean; notes: NoteRef[]; message?: string }
  | { kind: "proposal"; proposal: Proposal; state: ProposalState; error?: string }
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
      const summary = (d.summary ?? {}) as { notes?: NoteRef[]; message?: string };
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
        }),
      );
      break;
    }
    case "proposal":
      push({ kind: "proposal", proposal: d as unknown as Proposal, state: "open" });
      break;
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

function setProposal(turn: number, part: number, patch: { state: ProposalState; error?: string }) {
  state.setChat(
    "turns",
    turn,
    "parts",
    part,
    produce((p) => {
      if (p.kind !== "proposal") return;
      p.state = patch.state;
      p.error = patch.error;
    }),
  );
}

/** Apply a proposal: write its note through the notes API at the version it was based on. */
export async function applyProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "proposal" || (p.state !== "open" && p.state !== "failed")) return;
  setProposal(turn, part, { state: "applying" });
  try {
    await api.updateNote(p.proposal.note_id, p.proposal.after, p.proposal.base_version);
    setProposal(turn, part, { state: "applied" });
  } catch (e) {
    const error =
      e instanceof ApiError && e.isConflict
        ? "The note changed after this was proposed, so it was not applied. Ask again for a fresh proposal."
        : e instanceof Error
          ? e.message
          : String(e);
    setProposal(turn, part, { state: "failed", error });
  }
}

export function discardProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind === "proposal" && p.state !== "applied" && p.state !== "applying") setProposal(turn, part, { state: "discarded" });
}
