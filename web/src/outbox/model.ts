// The Outbox (outbox X7, X8): emails, SMS and calendar invites waiting to be sent. Items are
// notes of the built-in `outbox` type (crates/writer/defaults/outbox.wcl); the writer checks who
// may move an item from one status to another, and the server's `outbox_send` handler sends what
// becomes `approved`.

import type { Block, Json, NoteDoc } from "../api";
import { escapeText } from "../editor/markup";

/** The built-in type of an Outbox item. */
export const OUTBOX = "outbox";

/** The built-in types Reply and Propose a change start from. */
export const EMAIL = "email";
export const EVENT = "event";

export type Kind = "email" | "sms" | "event";

export type Status = "draft" | "review" | "approved" | "sending" | "sent" | "failed" | "rejected";

/** Who drafted an item the user made. */
export const DRAFTED_BY_USER = "user";

/** The Outbox page's tabs, each the statuses it lists. */
export const TABS: readonly { key: string; label: string; statuses: readonly Status[] }[] = [
  { key: "review", label: "Review", statuses: ["review"] },
  { key: "drafts", label: "Drafts", statuses: ["draft"] },
  { key: "sending", label: "Sending", statuses: ["approved", "sending"] },
  { key: "sent", label: "Sent", statuses: ["sent"] },
  { key: "failed", label: "Failed", statuses: ["failed"] },
  { key: "rejected", label: "Rejected", statuses: ["rejected"] },
];

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

/** The items waiting for the user's review: the side menu's badge counts them. */
export const REVIEW_QUERY = `from ${OUTBOX} where status == "review"`;

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

/** Who drafted an item, in words: "You" or the agent's name. */
export function draftedByText(fields: Record<string, Json>, label?: (name: string) => string | undefined): string {
  const by = str(fields, "drafted_by");
  if (!by || by === DRAFTED_BY_USER) return "You";
  return label?.(by) ?? by;
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

/**
 * A reply to email note `id` (outbox X8): a draft with `in_reply_to`, To the sender, Subject
 * `Re: ...` and the email's body quoted below an empty line to write in.
 */
export function replyDraft(id: string, email: NoteDoc): NoteDoc {
  const f = email.fields;
  const subject = replySubject(str(f, "title") || "(no subject)");
  const from = str(f, "from_raw");
  const to = from ? [addressOf(from)] : [];
  const quoted = markupLines(contentOf(email, "body"));
  const sent = str(f, "sent");
  const when = sent ? new Date(sent) : null;
  const intro = `${when && !Number.isNaN(when.getTime()) ? `On ${when.toLocaleString()}, ` : ""}${from || "they"} wrote:`;
  const body: Block[] = [paragraph("")];
  if (quoted.some((l) => l.trim())) {
    body.push(paragraph(escapeText(intro)), { kind: "quote", fields: { text: quoted.join("\n") } });
  }
  return {
    fields: {
      title: subject,
      kind: "email",
      status: "draft",
      drafted_by: DRAFTED_BY_USER,
      to,
      subject,
      in_reply_to: id,
    },
    blocks: [content("body", body)],
  };
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
