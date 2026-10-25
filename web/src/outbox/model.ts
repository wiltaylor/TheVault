// The Outbox (outbox X7, X8, X14): emails, SMS and calendar invites waiting to be sent. Items are
// notes of the built-in `outbox` type (crates/writer/defaults/outbox.wcl); the writer checks who
// may move an item from one status to another, and the server's `outbox_send` handler sends what
// becomes `ready`. A sent item leaves the Outbox: its copy is a regular email, message or event.

import type { Block, Json, NoteDoc } from "../api";
import { escapeText } from "../editor/markup";

/** The built-in type of an Outbox item. */
export const OUTBOX = "outbox";

/** The built-in types Reply, Forward and Propose a change start from. */
export const EMAIL = "email";
export const EVENT = "event";
export const MESSAGE = "message";

export type Kind = "email" | "sms" | "event";

export type Status = "draft" | "ready" | "sending" | "failed";

/** What each status says on an item, on the Outbox page and the item's own page. */
export const STATUS_TEXT: Record<Status, string> = {
  draft: "Draft",
  ready: "Ready",
  sending: "Sending",
  failed: "Error",
};

/** Who drafted an item the user made. */
export const DRAFTED_BY_USER = "user";

/** The Outbox page's tabs, each the statuses it lists (outbox X14). */
export const TABS: readonly { key: string; label: string; statuses: readonly Status[] }[] = [
  { key: "draft", label: "Draft", statuses: ["draft"] },
  { key: "ready", label: "Ready", statuses: ["ready", "sending"] },
  { key: "error", label: "Error", statuses: ["failed"] },
];

/** The Outbox's Draft tab, opened when one action makes several drafts. */
export const DRAFT_TAB_HREF = "/outbox?tab=draft";

/** The index in [`TABS`] of the tab `key` names, also as the older links name it, or -1. */
export function tabByKey(key: unknown): number {
  const k = key === "drafts" ? "draft" : key === "failed" ? "error" : key;
  return TABS.findIndex((t) => t.key === k);
}

/** The query of the items in `statuses`, newest change first. */
export function statusQuery(statuses: readonly Status[]): string {
  const where = statuses.map((s) => `status == "${s}"`).join(" || ");
  return `from ${OUTBOX} where ${where} sort updated desc, id desc`;
}

/** The Outbox page opened on item `id`, in the tab of its status. */
export function outboxItemHref(id: string): string {
  return `/outbox?item=${encodeURIComponent(id)}`;
}

/** The index in [`TABS`] of the tab that lists items of `status`. */
export function tabOf(status: string): number {
  return Math.max(
    0,
    TABS.findIndex((t) => (t.statuses as readonly string[]).includes(status)),
  );
}

/** The drafts an agent made, which wait on the user: the side menu's badge counts them. */
export const AGENT_DRAFTS_QUERY = `from ${OUTBOX} where status == "draft" && drafted_by != "${DRAFTED_BY_USER}"`;

/** A field's value as text, empty when it holds none. */
export function str(fields: Record<string, Json>, name: string): string {
  const v = fields[name];
  return typeof v === "string" ? v : "";
}

/** A list field's strings. */
export function strs(fields: Record<string, Json>, name: string): string[] {
  const v = fields[name];
  if (typeof v === "string") return v ? [v] : [];
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && x !== "") : [];
}

export function kindOf(fields: Record<string, Json>): Kind {
  const k = str(fields, "kind");
  return k === "sms" || k === "event" ? k : "email";
}

export function statusOf(fields: Record<string, Json>): Status {
  return (str(fields, "status") || "draft") as Status;
}

/** The agent that drafted an item, or null when the user did. */
export function draftingAgent(fields: Record<string, Json>): string | null {
  const by = str(fields, "drafted_by");
  return by && by !== DRAFTED_BY_USER ? by : null;
}

/** The blocks of content field `name`. */
export function contentOf(doc: NoteDoc, name: string): Block[] {
  return doc.blocks.find((b) => b.kind === "content" && b.labels?.[0] === name)?.blocks ?? [];
}

/** A content field `name` holding `blocks`. */
function content(name: string, blocks: Block[]): Block {
  return { kind: "content", labels: [name], blocks };
}

const paragraph = (text: string): Block => ({ kind: "paragraph", fields: { text } });

/** A new draft of `kind`, drafted by the user (outbox X8). */
export function newDraft(kind: Kind): NoteDoc {
  const title = { email: "New email", sms: "New SMS", event: "New event" }[kind];
  return {
    fields: { title, kind, status: "draft", drafted_by: DRAFTED_BY_USER },
    blocks: kind === "event" ? [] : [content("body", [paragraph("")])],
  };
}

/** The address in `Name <addr>`, or the text as it is. */
export function addressOf(raw: string): string {
  const m = /<([^<>\s]+@[^<>\s]+)>/.exec(raw);
  return (m ? m[1]! : raw).trim();
}

/** `subject` with one `Re: ` in front. */
export function replySubject(subject: string): string {
  const s = subject.trim();
  return /^re\s*:/i.test(s) ? s : `Re: ${s}`;
}

/** The raw text lines of `blocks`, still in inline markup, for quoting. */
function markupLines(blocks: readonly Block[]): string[] {
  const out: string[] = [];
  for (const b of blocks) {
    const t = b.fields?.text;
    if (typeof t === "string") out.push(...t.split("\n"));
    if (b.blocks) out.push(...markupLines(b.blocks));
  }
  return out;
}

/** `subject` with one `Fwd: ` in front. */
export function forwardSubject(subject: string): string {
  const s = subject.trim();
  return /^(fwd?|fw)\s*:/i.test(s) ? s : `Fwd: ${s}`;
}

/** `addresses` without empty ones, those in `leaveOut`, and repeats, compared ignoring case. */
function uniqueAddresses(addresses: readonly string[], leaveOut: readonly string[] = []): string[] {
  const seen = new Set(leaveOut.filter(Boolean).map((a) => a.toLowerCase()));
  const out: string[] = [];
  for (const a of addresses) {
    const key = a.toLowerCase();
    if (!a || seen.has(key)) continue;
    seen.add(key);
    out.push(a);
  }
  return out;
}

/** An email's `sent` as local text, or empty. */
function sentText(f: Record<string, Json>): string {
  const sent = str(f, "sent");
  const when = sent ? new Date(sent) : null;
  return when && !Number.isNaN(when.getTime()) ? when.toLocaleString() : "";
}

/** An email's body lines, still in inline markup, as a quote block, or nothing when empty. */
function quoteOf(email: NoteDoc): Block[] {
  const quoted = markupLines(contentOf(email, "body"));
  return quoted.some((l) => l.trim()) ? [{ kind: "quote", fields: { text: quoted.join("\n") } }] : [];
}

/** An email draft drafted by the user. */
function emailDraft(subject: string, fields: Record<string, Json>, body: Block[]): NoteDoc {
  return {
    fields: { title: subject, kind: "email", status: "draft", drafted_by: DRAFTED_BY_USER, ...fields, subject },
    blocks: [content("body", body)],
  };
}

/**
 * A reply to email note `id` (outbox X8, X13): a draft with `in_reply_to`, To the sender, Subject
 * `Re: ...` and the email's body quoted below an empty line to write in.
 */
export function replyDraft(id: string, email: NoteDoc): NoteDoc {
  const f = email.fields;
  const subject = replySubject(str(f, "title") || "(no subject)");
  const from = str(f, "from_raw");
  const to = from ? [addressOf(from)] : [];
  const when = sentText(f);
  const intro = `${when ? `On ${when}, ` : ""}${from || "they"} wrote:`;
  const quote = quoteOf(email);
  const body: Block[] = [paragraph("")];
  if (quote.length) body.push(paragraph(escapeText(intro)), ...quote);
  return emailDraft(subject, { to, in_reply_to: id }, body);
}

/**
 * A reply to everyone on email note `id` (outbox X13): [`replyDraft`] with To the sender, then the
 * original To and Cc, without `account` (the connected Google address) and without repeats,
 * ignoring case.
 */
export function replyAllDraft(id: string, email: NoteDoc, account?: string | null): NoteDoc {
  const doc = replyDraft(id, email);
  const f = email.fields;
  const everyone = [str(f, "from_raw"), ...strs(f, "to_raw"), ...strs(f, "cc_raw")].map(addressOf);
  const to = uniqueAddresses(everyone, account ? [addressOf(account)] : []);
  // An email the user sent only to themselves still replies to its sender.
  if (to.length) doc.fields.to = to;
  return doc;
}

/**
 * A forward of an email note (outbox X13): To empty, Subject `Fwd: ...`, the body an empty line
 * to write in, a "Forwarded message" header block (From, Date, Subject, To) and the email's body
 * quoted, and the email's attachments. It joins no thread.
 */
export function forwardDraft(email: NoteDoc): NoteDoc {
  const f = email.fields;
  const original = str(f, "title") || "(no subject)";
  const subject = forwardSubject(original);
  const header = [
    "---------- Forwarded message ---------",
    `From: ${str(f, "from_raw")}`,
    `Date: ${sentText(f)}`,
    `Subject: ${original}`,
    `To: ${strs(f, "to_raw").join(", ")}`,
  ];
  const body: Block[] = [paragraph(""), ...header.map((l) => paragraph(escapeText(l))), ...quoteOf(email)];
  const attachments = strs(f, "attachments");
  return emailDraft(subject, { to: [], ...(attachments.length ? { attachments } : {}) }, body);
}

/** Whether `text` reads as a phone number rather than a name. */
export function isPhoneNumber(text: string): boolean {
  return /^\+?[\d\s().-]{3,}$/.test(text.trim()) && /\d/.test(text);
}

/** A person's phones, each `{ number, label }` in its `phones` list: the numbers, with their labels ("" for none). */
export function phonesOf(person: NoteDoc): { number: string; label: string }[] {
  const v = person.fields.phones;
  return (Array.isArray(v) ? v : []).flatMap((p) => {
    const rec = p && typeof p === "object" && !Array.isArray(p) ? p : null;
    const number = rec ? rec.number : p;
    const label = rec ? rec.label : undefined;
    return typeof number === "string" && number ? [{ number, label: typeof label === "string" ? label : "" }] : [];
  });
}

/** The number to text person note `person` at: its first mobile number, else its first number. */
export function phoneOf(person: NoteDoc | null | undefined): string {
  if (!person) return "";
  const phones = phonesOf(person);
  return (phones.find((p) => p.label === "mobile") ?? phones[0])?.number ?? "";
}

/** An SMS draft to `phone` drafted by the user, titled `title`, with one paragraph per line. */
export function smsDraft(title: string, phone: string, lines: readonly string[] = [""]): NoteDoc {
  return {
    fields: { title, kind: "sms", status: "draft", drafted_by: DRAFTED_BY_USER, phone },
    blocks: [content("body", (lines.length ? lines : [""]).map((l) => paragraph(escapeText(l))))],
  };
}

/**
 * The other parties of message note `message` (outbox X13): the sender of one that came in (with
 * `all`, also the others it went to), else its recipients. `raws` are the numbers or names as the
 * phone gave them; `people` the person notes it links, which need not line up with `raws`.
 */
export function messageParties(message: NoteDoc, all = false): { raws: string[]; people: string[] } {
  const f = message.fields;
  if (str(f, "direction") !== "in") return { raws: strs(f, "to_raw"), people: strs(f, "to") };
  const raws = [str(f, "from_raw")].concat(all ? strs(f, "to_raw") : []).filter(Boolean);
  const people = [str(f, "from")].concat(all ? strs(f, "to") : []).filter(Boolean);
  return { raws, people };
}

/** A key comparing numbers by their digits. */
export function numberKey(n: string): string {
  return n.replace(/\D/g, "");
}

/** One number an SMS reply goes to, and whose it is. */
export interface SmsTarget {
  number: string;
  name: string;
}

/**
 * The numbers to reply at (outbox X13): each of `raws` that is a number, named by the person of
 * `people` holding it, then the number of each person no such number belongs to. Repeats,
 * compared by their digits, are left out.
 */
export function smsTargets(raws: readonly string[], people: readonly { doc: NoteDoc | null }[]): SmsTarget[] {
  const out: SmsTarget[] = [];
  const seen = new Set<string>();
  const add = (number: string, name: string) => {
    const key = numberKey(number);
    if (!key || seen.has(key)) return;
    seen.add(key);
    out.push({ number, name: name || number });
  };
  const holder = (number: string) =>
    people.find((p) => p.doc && phonesOf(p.doc).some((n) => numberKey(n.number) === numberKey(number)))?.doc;
  for (const raw of raws) {
    if (isPhoneNumber(raw)) add(raw.trim(), str(holder(raw)?.fields ?? {}, "title"));
  }
  for (const p of people) {
    if (!p.doc) continue;
    if (phonesOf(p.doc).some((n) => seen.has(numberKey(n.number)))) continue;
    add(phoneOf(p.doc), str(p.doc.fields, "title"));
  }
  return out;
}

/** An SMS reply to `to`, the body empty to write in. */
export function smsReplyDraft(to: SmsTarget): NoteDoc {
  const who = to.name || to.number;
  return smsDraft(who ? `SMS to ${who}` : "SMS reply", to.number);
}

/** A forward of message note `message` (outbox X13): no number yet, the body `Fwd: <text>`. */
export function smsForwardDraft(message: NoteDoc): NoteDoc {
  const lines = bodyText(message).split("\n");
  lines[0] = `Fwd: ${lines[0] ?? ""}`;
  return smsDraft("Forwarded SMS", "", lines);
}

/**
 * A proposed change to event note `id` (outbox X8): a draft event item referencing it, titled as
 * the event is. Its other event fields start empty, each meaning "leave as it is" until the user
 * fills it in.
 */
export function changeDraft(id: string, event: NoteDoc): NoteDoc {
  return {
    fields: {
      title: str(event.fields, "title") || "Untitled event",
      kind: "event",
      status: "draft",
      drafted_by: DRAFTED_BY_USER,
      event: id,
    },
    blocks: [],
  };
}

/** A date range's value as text: `start – end`, or the one end it has. */
export function rangeText(v: Json | undefined): string {
  if (v && typeof v === "object" && !Array.isArray(v)) {
    const start = typeof v.start === "string" ? v.start : "";
    const end = typeof v.end === "string" ? v.end : "";
    return start && end && start !== end ? `${start} – ${end}` : start || end;
  }
  return typeof v === "string" ? v : "";
}

/** One line of an event item's preview: what the field is now and what the item sets it to. */
export interface EventChange {
  field: string;
  label: string;
  before: string;
  after: string;
}

const EVENT_FIELDS: readonly { name: string; label: string }[] = [
  { name: "title", label: "Title" },
  { name: "when", label: "When" },
  { name: "location", label: "Location" },
  { name: "attendees_raw", label: "Attendees" },
  { name: "calendar", label: "Calendar" },
  { name: "description", label: "Description" },
];

/** The text of an event field, from a note's fields or its content. */
function eventValue(doc: NoteDoc, name: string): string {
  if (name === "description") return markupLines(contentOf(doc, name)).join(" ").replace(/\\(.)/g, "$1").trim();
  if (name === "when") return rangeText(doc.fields[name]);
  if (name === "attendees_raw") return strs(doc.fields, name).join(", ");
  return str(doc.fields, name);
}

/**
 * What an event item changes (outbox X1): each non-empty field, beside the referenced event's
 * value now (`before` is empty for a new event). The title of an item for an existing event shows
 * only when it differs from the event's.
 */
export function eventChanges(item: NoteDoc, event: NoteDoc | null): EventChange[] {
  const out: EventChange[] = [];
  for (const f of EVENT_FIELDS) {
    const after = eventValue(item, f.name);
    if (!after) continue;
    const before = event ? eventValue(event, f.name) : "";
    if (f.name === "title" && event && before === after) continue;
    out.push({ field: f.name, label: f.label, before, after });
  }
  return out;
}

/** The plain text of an item's body, for the email and SMS previews. */
export function bodyText(doc: NoteDoc): string {
  return markupLines(contentOf(doc, "body"))
    .map((l) => l.replace(/\\(.)/g, "$1"))
    .join("\n")
    .trim();
}
