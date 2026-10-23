// The AI chat panel's conversation (ai D10, D14, D23): what the panel shows, and the neutral
// transcript the server takes back each turn. It lives in this tab's memory only; nothing is
// stored in the vault. The chat runs on the home server, which holds the credentials and runs
// the note tools. In the web app a turn streams from POST /api/ai/chat. In the desktop and
// Android apps, whose webviews get whole bodies from the embedded server, a turn starts with
// POST /api/ai/chat/turns, which the embedded server forwards, and its events are polled from
// the embedded server's GET /api/app/ai/turns/{turn}, which reads the turn's stream from the
// home server. Edits come back as proposals the user applies with POST /api/ai/apply, on the
// home server in every client, or, for the kinds Settings > AI lets go ahead without asking, as
// `applied` events for changes already made. In an app such a change reaches the device's copy
// with a sync round, which the panel asks for.

import { createRoot, createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { type AppInfo, type Json, type NoteDoc, ApiError, type StreamEvent, api, streamEvents } from "../api";
import { noteChangedElsewhere, noteWritten } from "../changes";
import { appInfo } from "../inApp";
import { reloadDefinitions } from "../session";
import { refreshSync } from "../sync/store";

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
/** A web page a `web_search` step found, from its `tool_result` summary (ai D26). */
export interface WebLink {
  title: string;
  url: string;
}

export type Applied =
  | { id: string; op: NoteOp; note_id: string; title: Json; version: string }
  | { id: string; op: "type_change" | "type_create"; type_name: string };

/** A file the model asked to download into the vault as an asset (ai D27). */
export interface DownloadProposal {
  id: string;
  op: "download";
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
  | { kind: "tool"; id: string; name: string; input: Json; done: boolean; isError: boolean; notes: NoteRef[]; message?: string; asset?: AssetRef; links?: WebLink[]; web?: WebRef }
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
  const [app, setApp] = createSignal<AppInfo | null>(null);
  return { chat, setChat, app, setApp };
});

export const chat = state.chat;

let asked = false;

/**
 * Reactive: the desktop or Android app the page runs in, or null in the web app and until the
 * answer is known. The panel shows in both; in an app it uses the turns form (ai D23).
 */
export function inApp(): AppInfo | null {
  if (!asked) {
    asked = true;
    void appInfo().then(
      (a) => state.setApp(a),
      () => undefined,
    );
  }
  return state.app();
}

/** Test-only: ask again whether the page runs in an app. */
export function forgetApp() {
  asked = false;
  state.setApp(null);
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
      const summary = (d.summary ?? {}) as { notes?: NoteRef[]; message?: string; asset?: AssetRef; links?: WebLink[]; web?: WebRef };
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
          if (Array.isArray(summary.links))
            p.links = summary.links
              .filter((l) => l && typeof l.url === "string" && /^https?:\/\//i.test(l.url))
              .map((l) => ({ url: l.url, title: String(l.title || l.url) }));
          if (summary.web && typeof summary.web.url === "string") p.web = summary.web;
        }),
      );
      break;
    }
    case "proposal":
      if (d.op === "download") {
        push({ kind: "download", proposal: d as unknown as DownloadProposal, state: "open" });
        break;
      }
      push({ kind: "proposal", proposal: { ...d, op: typeof d.op === "string" ? d.op : "update" } as unknown as Proposal, state: "open" });
      break;
    case "type_proposal":
      push({ kind: "type_proposal", proposal: { ...(d as unknown as TypeProposal), issues: Array.isArray(d.issues) ? (d.issues as TypeProposal["issues"]) : [] }, state: "open" });
      break;
    case "applied": {
      if (d.op === "download") {
        const asset = d as unknown as DownloadedAsset & { url?: string };
        push({
          kind: "download",
          proposal: { id: String(d.id), op: "download", url: asset.url ?? "", filename: asset.filename, media_type: asset.media_type ?? null, size: asset.size ?? null },
          state: "applied",
          asset,
          auto: true,
        });
        break;
      }
      const applied = d as unknown as Applied;
      push({ kind: "applied", applied });
      // Open views of the note or type take the change, as after a write of this client's own.
      void settle(applied.op === "type_change" || applied.op === "type_create" ? "type" : "note");
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
  const each = (e: StreamEvent) => mine === generation && onEvent(e);
  try {
    if (await appInfo().catch(() => null)) await pollTurn(body, each, ctl.signal);
    else await streamEvents("/api/ai/chat", body, each, ctl.signal);
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

/**
 * A turn in the turns form, as the apps run it (ai D23): started on the home server through the
 * embedded server, its events polled from the embedded server until the turn ends. Aborting
 * `signal` stops the turn on the home server.
 */
async function pollTurn(body: unknown, onEvent: (e: StreamEvent) => void, signal: AbortSignal) {
  const turn = await api.aiTurnStart(body);
  const stopTurn = () => void api.aiTurnStop(turn).catch(() => undefined);
  if (signal.aborted) stopTurn();
  signal.addEventListener("abort", stopTurn, { once: true });
  let after = 0;
  try {
    for (;;) {
      signal.throwIfAborted();
      const got = await api.aiTurnPoll(turn, after, signal);
      signal.throwIfAborted();
      for (const e of got.events) onEvent(e);
      after = got.next;
      if (got.done) return;
    }
  } finally {
    signal.removeEventListener("abort", stopTurn);
  }
}

/** How long the app waits for the sync round that brings a change made on the server. */
export const SETTLE_MS = 20_000;
/** How often the app asks whether that round is over. */
export const SETTLE_POLL_MS = 500;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * After a change the AI made on the home server, through Apply or itself: open views of the note
 * or type take it. In an app the change reaches the device's copy only by sync, so the app asks
 * its embedded server for a round first and waits for it to end (ai D23).
 */
export async function settle(kind: "note" | "type") {
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
    if (kind === "note") noteChangedElsewhere(null);
  }
  if (kind === "type") await reloadDefinitions().catch(() => undefined);
  else noteWritten();
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

/** What a card says when the note changed after the proposal (ai D20, D23). */
export const NOTE_CHANGED = "This note changed since the proposal, so it was not applied. Ask again for a fresh proposal.";

/** What a card says when the note the proposal names is gone. */
export const NOTE_GONE = "This note is no longer there, so nothing was applied.";

/**
 * Apply a note proposal with POST /api/ai/apply, on the home server, as the chat sent it (ai
 * D23): an update, an archive or unarchive, or a delete, which moves the note to the trash, at
 * the version it was proposed on, or a create. A note that changed since is a conflict and
 * nothing is written.
 */
export async function applyProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "proposal" || (p.state !== "open" && p.state !== "failed")) return;
  const proposal = p.proposal;
  setProposal(turn, part, { state: "applying" });
  try {
    const done = await api.aiApply(proposal);
    const created = proposal.op === "create" && "note_id" in done ? done.note_id : undefined;
    setProposal(turn, part, { state: "applied", ...(created ? { created } : {}) });
    await settle("note");
  } catch (e) {
    if (e instanceof ApiError && e.isConflict) setProposal(turn, part, { state: "stale", error: NOTE_CHANGED });
    else if (e instanceof ApiError && e.status === 404) setProposal(turn, part, { state: "stale", error: NOTE_GONE });
    else setProposal(turn, part, { state: "failed", error: e instanceof Error ? e.message : String(e) });
  }
}

/** Whether a part is a proposal still waiting for the user: open, or failed and retryable. */
export function pending(p: Part): boolean {
  return (p.kind === "proposal" || p.kind === "type_proposal" || p.kind === "download") && (p.state === "open" || p.state === "failed");
}

function setDownload(turn: number, part: number, patch: { state: ProposalState; error?: string; asset?: DownloadedAsset }) {
  state.setChat(
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
export async function applyDownload(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "download" || (p.state !== "open" && p.state !== "failed")) return;
  setDownload(turn, part, { state: "applying" });
  try {
    const done = await api.aiApply(p.proposal);
    const asset = "asset_id" in done ? done : undefined;
    setDownload(turn, part, { state: "applied", ...(asset ? { asset } : {}) });
    await settle("note");
  } catch (e) {
    setDownload(turn, part, { state: "failed", error: e instanceof Error ? e.message : String(e) });
  }
}

export function discardDownload(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind === "download" && p.state !== "applied" && p.state !== "applying") setDownload(turn, part, { state: "discarded" });
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
    else if (p.kind === "download") await applyDownload(turn, i);
    else await applyTypeProposal(turn, i);
    const after = state.chat.turns[turn]?.parts[i];
    if (!after || (after.kind !== "proposal" && after.kind !== "type_proposal" && after.kind !== "download") || after.state !== "applied") return;
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

/**
 * Apply a type proposal with POST /api/ai/apply, on the home server (ai D18, D23): a new type, or
 * a change planned and applied as one commit, when the type file is still the one proposed
 * against. It needs admin: from an app, the server answers that it is done in the web app.
 */
export async function applyTypeProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind !== "type_proposal" || (p.state !== "open" && p.state !== "failed")) return;
  setTypeProposal(turn, part, { state: "applying" });
  try {
    await api.aiApply(p.proposal);
    setTypeProposal(turn, part, { state: "applied" });
    await settle("type");
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) setTypeProposal(turn, part, { state: "stale", error: TYPE_CHANGED });
    else setTypeProposal(turn, part, { state: "failed", error: e instanceof Error ? e.message : String(e) });
  }
}

export function discardTypeProposal(turn: number, part: number) {
  const p = state.chat.turns[turn]?.parts[part];
  if (p?.kind === "type_proposal" && p.state !== "applied" && p.state !== "applying") setTypeProposal(turn, part, { state: "discarded" });
}
