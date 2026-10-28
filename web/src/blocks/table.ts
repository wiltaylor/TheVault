// `table` content blocks (docs/vault-format.md, "table"): rows of inline markup cells, an
// optional header row and an alignment per column. Also the table's Markdown: GitHub-flavoured
// Markdown tables read from pasted text, and written for the clipboard, with each cell's inline
// Markdown converted to and from the vault's inline markup (PRD Appendix A).

import { Lexer, type Token, type Tokens } from "marked";
import type { Block, Json } from "../api";
import { type InlineNode, type Mark, isUlid, parseInline, serializeInline } from "../editor/markup";

export const TABLE_ALIGNS = ["left", "center", "right"] as const;
export type Align = (typeof TABLE_ALIGNS)[number];

/** The bounds `schema` holds a table to: columns in its widest row, and rows. */
export const TABLE_COLUMNS_MAX = 100;
export const TABLE_ROWS_MAX = 2000;

/** A table as the client works with it: every row as long as the widest, an alignment per column. */
export interface TableData {
  header: boolean;
  align: Align[];
  /** The cells, row by row, as inline markup. */
  rows: string[][];
}

export function isAlign(v: unknown): v is Align {
  return typeof v === "string" && (TABLE_ALIGNS as readonly string[]).includes(v);
}

/** The cells of a table's `row` block, as markup strings. */
export function cellsOf(row: Block): string[] {
  const cells = row.fields?.cells;
  return Array.isArray(cells) ? cells.map((c) => (typeof c === "string" ? c : "")) : [];
}

/** A `table` block read square: short rows padded with empty cells, `align` one per column. */
export function tableOf(block: Block): TableData {
  const rows = (block.blocks ?? []).filter((b) => b.kind === "row").map(cellsOf);
  const width = Math.max(1, ...rows.map((r) => r.length));
  const align = Array.isArray(block.fields?.align) ? block.fields.align : [];
  return {
    header: block.fields?.header === true,
    align: Array.from({ length: width }, (_, i) => (isAlign(align[i]) ? align[i] : "left")),
    rows: (rows.length ? rows : [[]]).map((r) => Array.from({ length: width }, (_, i) => r[i] ?? "")),
  };
}

/** The `table` block of `t`. `align` is written only when a column is not aligned left. */
export function tableBlock(t: TableData): Block {
  const width = Math.max(1, ...t.rows.map((r) => r.length));
  const fields: Record<string, Json> = {};
  if (t.header) fields.header = true;
  const align = Array.from({ length: width }, (_, i) => t.align[i] ?? "left");
  if (align.some((a) => a !== "left")) fields.align = align;
  const rows = t.rows.map((r) => ({ kind: "row", fields: { cells: Array.from({ length: width }, (_, i) => r[i] ?? "") } }));
  return Object.keys(fields).length ? { kind: "table", fields, blocks: rows } : { kind: "table", blocks: rows };
}

/** A new table of `rows` by `cols` empty cells, its first row the header. */
export function newTable(rows = 3, cols = 3): TableData {
  return { header: true, align: Array<Align>(cols).fill("left"), rows: Array.from({ length: rows }, () => Array<string>(cols).fill("")) };
}

// ---------------------------------------------------------------------------------------------
// Reading Markdown tables

/** The cells of one line of a Markdown table: split at each `|` not escaped, outer pipes optional. */
export function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const cells: string[] = [];
  let cell = "";
  for (let i = 0; i < s.length; i++) {
    const c = s.charAt(i);
    if (c === "\\" && s.charAt(i + 1) === "|") {
      // An escaped pipe is a pipe in the cell, as GFM reads it even inside code.
      cell += "|";
      i++;
    } else if (c === "|") {
      cells.push(cell.trim());
      cell = "";
    } else {
      cell += c;
    }
  }
  cells.push(cell.trim());
  return cells;
}

const DELIMITER = /^:?-+:?$/;

/** The alignments of a delimiter line such as `| :--- | ---: |`, or null when it is not one. */
function delimiterRow(line: string): Align[] | null {
  if (!line.includes("-") || !/^[\s|:-]+$/.test(line)) return null;
  const cells = splitRow(line);
  if (!cells.every((c) => DELIMITER.test(c))) return null;
  return cells.map((c) => (c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left"));
}

/** A line that may be a row of a table: it holds a pipe and is not blank. */
const rowLike = (line: string) => line.trim() !== "" && line.includes("|");

/** Pasted text in pieces: runs of lines that are not a table, and the tables between them. */
export type Piece = { kind: "text"; lines: string[] } | { kind: "table"; table: TableData };

/**
 * The tables in Markdown `text` (GFM: a header line, a delimiter line of `---` with optional
 * colons for alignment and as many cells, then body lines until a blank line or one with no
 * pipe), with the lines around them; null when it holds no table. Each cell's inline Markdown
 * becomes inline markup. A row longer than the header widens the table rather than losing cells.
 */
export function markdownPieces(text: string): Piece[] | null {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const pieces: Piece[] = [];
  let loose: string[] = [];
  let found = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const align = i + 1 < lines.length && rowLike(line) ? delimiterRow(lines[i + 1]!) : null;
    const head = align ? splitRow(line) : null;
    if (!align || !head || head.length !== align.length || delimiterRow(line)) {
      loose.push(line);
      continue;
    }
    let j = i + 2;
    const body: string[][] = [];
    while (j < lines.length && rowLike(lines[j]!)) body.push(splitRow(lines[j++]!));
    if (loose.length) pieces.push({ kind: "text", lines: loose });
    loose = [];
    const width = Math.max(head.length, ...body.map((r) => r.length));
    const square = (r: string[]) => Array.from({ length: width }, (_, k) => markdownToMarkup(r[k] ?? ""));
    pieces.push({
      kind: "table",
      table: { header: true, align: Array.from({ length: width }, (_, k) => align[k] ?? "left"), rows: [square(head), ...body.map(square)] },
    });
    found = true;
    i = j - 1;
  }
  if (!found) return null;
  // The line break that ends pasted text is not a line of its own.
  while (loose.length && loose.at(-1)!.trim() === "") loose.pop();
  if (loose.length) pieces.push({ kind: "text", lines: loose });
  return pieces;
}

// ---------------------------------------------------------------------------------------------
// Inline Markdown to inline markup

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: "\u00a0" };

function decodeEntities(s: string): string {
  return s.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (m, e: string) => {
    if (e.startsWith("#x") || e.startsWith("#X")) return String.fromCodePoint(parseInt(e.slice(2), 16));
    if (e.startsWith("#")) return String.fromCodePoint(Number(e.slice(1)));
    return ENTITIES[e.toLowerCase()] ?? m;
  });
}

/** Inline HTML tags read as marks: `<u>`, `<mark>`, `<s>` and the bold and italic ones. */
const HTML_MARKS: Record<string, Mark["type"]> = {
  u: "underline",
  ins: "underline",
  mark: "highlight",
  s: "strike",
  del: "strike",
  strike: "strike",
  b: "bold",
  strong: "bold",
  i: "italic",
  em: "italic",
  code: "code",
};

/** A link's target that is a note of this vault, `/notes/<ULID>` on any host. */
function noteLink(href: string): string | null {
  const m = /(?:^|\/)notes\/([0-9A-HJKMNP-TV-Z]{26})\/?$/.exec(href);
  return m && isUlid(m[1]!) ? m[1]! : null;
}

/** The plain text of tokens, for a mention's label. */
function plainOf(tokens: Token[] | undefined): string {
  return (tokens ?? []).map((t) => ("tokens" in t && t.tokens ? plainOf(t.tokens) : "text" in t ? decodeEntities(String(t.text)) : "")).join("");
}

/** `[[ULID]]` and `[[ULID|label]]` in Markdown text are mentions, as the vault writes them. */
const MENTION = /\[\[([0-9A-HJKMNP-TV-Z]{26})(?:\|((?:\\.|[^\]\\])*))?\]\]/g;

/** Inline Markdown (GFM) as inline markup: emphasis, code, strikethrough, links, line breaks. */
export function markdownToMarkup(md: string): string {
  const nodes: InlineNode[] = [];
  const html = new Set<Mark["type"]>();
  const withMarks = (marks: Mark[]): Mark[] | undefined => {
    const all = [...marks, ...[...html].filter((t) => !marks.some((m) => m.type === t)).map((type) => ({ type }) as Mark)];
    return all.length ? all : undefined;
  };
  const text = (t: string, marks: Mark[]) => {
    if (!t) return;
    const m = withMarks(marks);
    nodes.push(m ? { type: "text", text: t, marks: m } : { type: "text", text: t });
  };
  const plain = (t: string, marks: Mark[]) => {
    let at = 0;
    for (const m of t.matchAll(MENTION)) {
      text(t.slice(at, m.index), marks);
      const label = m[2] === undefined ? null : m[2].replace(/\\(.)/g, "$1");
      nodes.push({ type: "mention", attrs: { id: m[1]!, label } });
      at = m.index + m[0].length;
    }
    text(t.slice(at), marks);
  };
  const walk = (tokens: Token[], marks: Mark[]) => {
    for (const t of tokens) {
      switch (t.type) {
        case "text":
          if ("tokens" in t && t.tokens?.length) walk(t.tokens, marks);
          else plain(decodeEntities((t as Tokens.Text).text), marks);
          break;
        case "escape":
          text((t as Tokens.Escape).text, marks);
          break;
        case "strong":
          walk((t as Tokens.Strong).tokens, [...marks, { type: "bold" }]);
          break;
        case "em":
          walk((t as Tokens.Em).tokens, [...marks, { type: "italic" }]);
          break;
        case "del":
          walk((t as Tokens.Del).tokens, [...marks, { type: "strike" }]);
          break;
        case "codespan":
          text(decodeEntities((t as Tokens.Codespan).text), [...marks, { type: "code" }]);
          break;
        case "br":
          nodes.push({ type: "hardBreak" });
          break;
        case "link": {
          const link = t as Tokens.Link;
          const note = noteLink(link.href);
          if (note) nodes.push({ type: "mention", attrs: { id: note, label: plainOf(link.tokens) || null } });
          else walk(link.tokens, [...marks, { type: "link", attrs: { href: link.href } }]);
          break;
        }
        case "image": {
          const image = t as Tokens.Image;
          text(image.text || image.href, [...marks, { type: "link", attrs: { href: image.href } }]);
          break;
        }
        case "html": {
          const tag = /^<\s*(\/)?\s*([a-z]+)[^>]*>$/i.exec(t.raw.trim());
          const name = tag?.[2]?.toLowerCase() ?? "";
          if (name === "br") nodes.push({ type: "hardBreak" });
          else if (HTML_MARKS[name]) {
            if (tag![1]) html.delete(HTML_MARKS[name]!);
            else html.add(HTML_MARKS[name]!);
          } else text(t.raw, marks);
          break;
        }
        default:
          text("raw" in t ? String(t.raw) : "", marks);
      }
    }
  };
  walk(Lexer.lexInline(md), []);
  return serializeInline(nodes);
}

// ---------------------------------------------------------------------------------------------
// Inline markup to inline Markdown, and tables to Markdown

/** Characters Markdown would read as syntax in plain text, escaped with a backslash. */
const MD_SPECIAL = /[\\`*_[\]<>~|]/g;

/** The title of a note, for a mention with no label, when it is known. */
export type TitleOf = (id: string) => string | null | undefined;

export interface MarkdownOptions {
  titleOf?: TitleOf;
  /** The text is a table cell's: a pipe in code is escaped too, as GFM reads one there. */
  cell?: boolean;
}

const MD_RANK = ["link", "bold", "italic", "strike", "underline", "highlight", "code"] as const;

function mdOpen(m: Mark): string {
  switch (m.type) {
    case "bold":
      return "**";
    case "italic":
      return "*";
    case "strike":
      return "~~";
    case "underline":
      return "<u>";
    case "highlight":
      return "<mark>";
    case "link":
      return "[";
    default:
      return "";
  }
}

function mdClose(m: Mark): string {
  switch (m.type) {
    case "underline":
      return "</u>";
    case "highlight":
      return "</mark>";
    case "link":
      // Brackets and spaces in the address would end it early.
      return `](${m.attrs.href.replace(/[()\s]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0")}`)})`;
    default:
      return mdOpen(m);
  }
}

const markId = (m: Mark) => (m.type === "link" ? `link:${m.attrs.href}` : m.type);

/** A code span holding `t`, its fence longer than any run of backticks inside. */
function codeSpan(t: string, cell: boolean): string {
  if (cell) t = t.replace(/\|/g, "\\|");
  const longest = Math.max(0, ...[...t.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = "`".repeat(longest + 1);
  const pad = t.startsWith("`") || t.endsWith("`") ? " " : "";
  return `${fence}${pad}${t}${pad}${fence}`;
}

/**
 * Inline markup as GFM inline Markdown: bold, italic, strikethrough, code and links as Markdown,
 * underline and highlight as `<u>` and `<mark>`, colour dropped. A mention is a link to the note
 * at `/notes/<ULID>` labelled with its label or the note's title, or `[[ULID]]` when neither is
 * known; a placeholder link is its title. Whitespace at the edge of a marked run sits outside it.
 */
export function markupToMarkdown(markup: string, options: MarkdownOptions = {}): string {
  const { titleOf, cell = false } = options;
  let out = "";
  let open: Mark[] = [];
  let code = "";
  const flushCode = () => {
    if (code) out += codeSpan(code, cell);
    code = "";
  };
  const step = (want: Mark[]) => {
    const keys = want.map(markId);
    // Close from the innermost out to the first mark that is not kept.
    let keep = 0;
    while (keep < open.length && keys.includes(markId(open[keep]!))) keep++;
    for (let k = open.length - 1; k >= keep; k--) {
      if (open[k]!.type === "code") flushCode();
      else out += mdClose(open[k]!);
    }
    open = open.slice(0, keep);
    for (const m of want) {
      if (open.some((o) => markId(o) === markId(m))) continue;
      if (m.type !== "code") out += mdOpen(m);
      open.push(m);
    }
  };
  const sorted = (marks: Mark[] | undefined) =>
    (marks ?? [])
      .filter((m) => m.type !== "colour")
      .sort((a, b) => MD_RANK.indexOf(a.type as (typeof MD_RANK)[number]) - MD_RANK.indexOf(b.type as (typeof MD_RANK)[number]));
  const nodes = parseInline(markup);
  const shared = (marks: Mark[]) => open.filter((m) => marks.some((w) => markId(w) === markId(m)));
  for (const [n, node] of nodes.entries()) {
    const marks = sorted(node.marks);
    if (node.type === "text") {
      const inCode = marks.some((m) => m.type === "code");
      const [, lead, core, trail] = /^(\s*)([\s\S]*?)(\s*)$/.exec(node.text)!;
      if (inCode) {
        step(marks);
        code += node.text;
        continue;
      }
      // Whitespace at the edges goes outside the marks that start or end there.
      step(shared(marks));
      out += lead;
      if (!core) continue;
      step(marks);
      out += core!.replace(MD_SPECIAL, "\\$&");
      if (trail) {
        step(shared(sorted(nodes[n + 1]?.marks)));
        out += trail;
      }
      continue;
    }
    step(marks.filter((m) => m.type !== "code"));
    if (node.type === "hardBreak") out += "<br>";
    else if (node.type === "mention") {
      const label = node.attrs.label || titleOf?.(node.attrs.id) || "";
      out += label ? `[${label.replace(MD_SPECIAL, "\\$&")}](/notes/${node.attrs.id})` : `[[${node.attrs.id}]]`;
    } else if (node.type === "placeholderLink") out += node.attrs.title.replace(MD_SPECIAL, "\\$&");
    else if (node.type === "fieldRef") out += `{{${node.attrs.name}}}`;
  }
  step([]);
  return out;
}

/** One cell's Markdown inside a table line, its pipes escaped and its line breaks `<br>`. */
const mdCell = (markup: string, titleOf?: TitleOf) => markupToMarkdown(markup, { titleOf, cell: true });

/**
 * `t` as a GFM Markdown table: the header row, a delimiter line carrying the alignments, then the
 * body. A table with no header row gets an empty one, since a Markdown table always has one.
 */
export function tableToMarkdown(t: TableData, titleOf?: TitleOf): string {
  const width = Math.max(1, ...t.rows.map((r) => r.length));
  const line = (cells: string[]) => `| ${Array.from({ length: width }, (_, i) => mdCell(cells[i] ?? "", titleOf)).join(" | ")} |`;
  const delimiter = `| ${Array.from({ length: width }, (_, i) => ({ left: "---", center: ":---:", right: "---:" })[t.align[i] ?? "left"]).join(" | ")} |`;
  const [head, ...body] = t.header ? t.rows : [Array<string>(width).fill(""), ...t.rows];
  return [line(head ?? []), delimiter, ...body.map(line)].join("\n");
}
