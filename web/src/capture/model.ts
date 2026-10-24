// Fleeting notes: what a capture becomes, and how a fleeting note turns into a note of another
// type. The built-in `fleeting` type is defined in crates/writer/defaults/fleeting.wcl.

import { agentCallHref } from "../agentCalls";
import type { Block, FieldDef, Json, NoteDoc, TypeDef } from "../api";
import { serializeInline } from "../editor/markup";

/** The built-in type of a quick capture. */
export const FLEETING = "fleeting";

/** The bool field that takes a fleeting note out of the inbox. */
export const PROCESSED = "processed";

/** The inbox: fleeting notes not processed yet. */
export const INBOX_QUERY = `from ${FLEETING} where !${PROCESSED}`;

/** The longest title a capture's first line gives, in characters, before it is cut. */
export const TITLE_MAX = 80;

const pad = (n: number) => String(n).padStart(2, "0");

/** `d` as `YYYY-MM-DD HH:MM` in the browser's own time zone. */
export function localDateTime(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * The title of a capture: its first line of text with its spaces run together, cut at a word to
 * at most TITLE_MAX characters; or, with no text, `Image <local date and time>`, or `Drawing
 * <local date and time>` for a capture of drawings alone.
 */
export function captureTitle(text: string, now: Date = new Date(), drawingOnly = false): string {
  const line = text
    .split("\n")
    .map((l) => l.replace(/\s+/g, " ").trim())
    .find((l) => l !== "");
  if (!line) return `${drawingOnly ? "Drawing" : "Image"} ${localDateTime(now)}`;
  const chars = [...line];
  if (chars.length <= TITLE_MAX) return line;
  const cut = chars.slice(0, TITLE_MAX - 1).join("");
  const space = cut.lastIndexOf(" ");
  return `${(space > TITLE_MAX / 2 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** Plain text as paragraph blocks: a blank line starts a paragraph, a newline breaks a line. */
export function textBlocks(text: string): Block[] {
  return text
    .replace(/\r\n?/g, "\n")
    .split(/\n[ \t]*\n/)
    .map((p) => p.replace(/^\n+|\s+$/g, ""))
    .filter((p) => p.trim() !== "")
    .map((p) => ({ kind: "paragraph", fields: { text: serializeInline([{ type: "text", text: p }]) } }));
}

/** The type's first field of `kind`, preferring the one called `name`. */
function fieldOf(type: TypeDef | undefined, kind: string, name?: string): FieldDef | undefined {
  const fields = (type?.fields ?? []).filter((f) => f.kind === kind && !f.computed);
  return fields.find((f) => f.name === name) ?? fields[0];
}

const field = (name: string, kind: string, multiple: boolean): FieldDef => ({
  name,
  label: name,
  description: null,
  kind,
  required: false,
  computed: false,
  multiple,
  compound: kind === "content",
});

/** The built-in type's fields, for when the types have not loaded. */
const DEFAULT_FIELDS = { body: field("body", "content", false), image: field("image", "asset", true) };

/**
 * Where a capture's text and images go in `type`: its `body` and `image` fields, or the first of
 * each kind. Before the types load, the built-in type's own fields.
 */
export function captureFields(type: TypeDef | undefined): { body?: FieldDef; image?: FieldDef } {
  if (!type) return DEFAULT_FIELDS;
  return { body: fieldOf(type, "content", "body"), image: fieldOf(type, "asset", "image") };
}

/** The request of the call that sends a capture to an agent (capture K3). */
export const CALL_REQUEST = "Process this capture";

/** The agent a capture is sent to, and the id of the call that sends it. */
export interface CaptureCall {
  agent: string;
  label: string;
  call: string;
}

/**
 * The paragraph that sends a capture to an agent: one committed call (agents G19),
 * `[@<Label> Process this capture](agent:<name>?call=<id>)`.
 */
export function callParagraph(c: CaptureCall): Block {
  const href = agentCallHref(c.agent, c.call);
  const text = serializeInline([{ type: "text", text: `@${c.label} ${CALL_REQUEST}`, marks: [{ type: "link", attrs: { href } }] }]);
  return { kind: "paragraph", fields: { text } };
}

/** What a capture holds besides its text and images. */
export interface CaptureExtras {
  /** `ink` content blocks drawn with the pen, which follow the text in the body (capture K2). */
  drawings?: Block[];
  /** The agent the capture is sent to. Its call follows everything else in the body (capture K3). */
  call?: CaptureCall | null;
}

/**
 * The note a capture of `text`, the uploaded `assets` and `extras` becomes, in the fleeting
 * `type`: the text's paragraphs, then the drawings, then the call, in its body.
 */
export function captureDoc(
  type: TypeDef | undefined,
  text: string,
  assets: string[],
  now: Date = new Date(),
  extras: CaptureExtras = {},
): NoteDoc {
  const { body, image } = captureFields(type);
  const drawings = extras.drawings ?? [];
  const fields: Record<string, Json> = { title: captureTitle(text, now, drawings.length > 0 && assets.length === 0) };
  if (!type || type.fields.some((f) => f.name === PROCESSED)) fields[PROCESSED] = false;
  if (image && assets.length) fields[image.name] = image.multiple ? assets : assets[0]!;
  const blocks: Block[] = [];
  const content = [...textBlocks(text), ...drawings, ...(extras.call ? [callParagraph(extras.call)] : [])];
  if (body && content.length) blocks.push({ kind: "content", labels: [body.name], blocks: content });
  return { fields, blocks };
}

/** Whether a capture has anything to save: text, or any image or drawing. */
export function hasContent(text: string, attachments: number): boolean {
  return text.trim() !== "" || attachments > 0;
}

/** The asset ids a note holds in `field`, a single or multiple asset field. */
export function assetIds(value: Json | undefined): string[] {
  if (typeof value === "string") return value ? [value] : [];
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
}

/** The plain text of a content field's blocks, for a preview: one line per block. */
export function blocksText(blocks: readonly Block[] | undefined): string {
  const out: string[] = [];
  const walk = (bs: readonly Block[]) => {
    for (const b of bs) {
      const t = b.fields?.text;
      if (typeof t === "string" && t.trim()) out.push(t.replace(/\\(.)/g, "$1"));
      if (b.blocks) walk(b.blocks);
    }
  };
  walk(blocks ?? []);
  return out.join("\n");
}

/**
 * The note of `target` type a fleeting note turns into: the same title, its body in the target's
 * first content field and its images in the target's first asset field, where the target has
 * them. The server gives every other field its default.
 */
export function convertDoc(from: NoteDoc, fromType: TypeDef | undefined, target: TypeDef): NoteDoc {
  const src = captureFields(fromType);
  const dst = { body: fieldOf(target, "content"), image: fieldOf(target, "asset") };
  const fields: Record<string, Json> = { title: from.fields.title ?? "Untitled" };
  const images = src.image ? assetIds(from.fields[src.image.name]) : [];
  if (dst.image && images.length) fields[dst.image.name] = dst.image.multiple ? images : images[0]!;
  const blocks: Block[] = [];
  const body = src.body && from.blocks.find((b) => b.kind === "content" && b.labels?.[0] === src.body!.name);
  if (dst.body && body?.blocks?.length) blocks.push({ ...body, labels: [dst.body.name] });
  return { fields, blocks };
}
