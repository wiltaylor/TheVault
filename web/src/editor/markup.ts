// Inline markup (PRD Appendix A, D41): the text of a heading, paragraph, list item or quote,
// parsed into ProseMirror's inline JSON and written back.
//
// Marks are a set, not a stack. Between two runs of content the writer emits the delimiters of
// every mark that ends, then of every mark that starts, so a delimiter always toggles its mark:
// a run of `*` one long toggles italic, two long bold, three long both. That keeps `***` and
// friends unambiguous without any flanking rules. Code is a literal span: it closes whenever any
// other mark changes and reopens after.

export const PALETTE = ["gray", "brown", "orange", "yellow", "green", "blue", "purple", "pink", "red"] as const;
export type Colour = (typeof PALETTE)[number];

export type Mark =
  | { type: "bold" | "italic" | "underline" | "strike" | "highlight" | "code" }
  | { type: "colour"; attrs: { colour: string } }
  | { type: "link"; attrs: { href: string } };

export type InlineNode =
  | { type: "text"; text: string; marks?: Mark[] }
  | { type: "mention"; attrs: { id: string; label: string | null }; marks?: Mark[] }
  | { type: "placeholderLink"; attrs: { noteType: string; title: string }; marks?: Mark[] }
  | { type: "hardBreak"; marks?: Mark[] }
  | { type: "fieldRef"; attrs: { name: string }; marks?: Mark[] };

/** Options for [`parseInline`]. */
export interface ParseOptions {
  /**
   * Read `{{name}}` as a field reference. Only a template's fixed blocks have them
   * (docs/vault-format.md, "Field references"); in a note `{{name}}` is text.
   */
  fieldRefs?: boolean;
}

/** `{{name}}` at the start of a string: a field reference, when parsing a template's text. */
const FIELD_REF = /^\{\{([A-Za-z_][A-Za-z0-9_]*)\}\}/;

/** The item of inline markup of each content block that has one: `text`, or an admonition's `title`. */
export const MARKUP_ITEMS: Readonly<Record<string, string>> = {
  heading: "text",
  paragraph: "text",
  item: "text",
  quote: "text",
  toggle: "text",
  admonition: "title",
};

/** Mark order: outermost first when opening. Code is always innermost. */
const RANK = ["link", "colour", "bold", "italic", "underline", "strike", "highlight", "code"] as const;
type MarkType = (typeof RANK)[number];

const TOGGLES: Record<string, MarkType> = { "++": "underline", "~~": "strike", "==": "highlight" };

/** Characters a backslash escapes. */
const MARKUP_CHARS = new Set("\\*+~`={}[]()|");

const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/;

export function isUlid(s: string): boolean {
  return ULID.test(s);
}

/** A type name: `[a-z][a-z0-9_]*`, at most 64 characters. */
const TYPE_NAME = /^[a-z][a-z0-9_]{0,63}$/;

function markKey(m: Mark): string {
  if (m.type === "link") return `link:${m.attrs.href}`;
  if (m.type === "colour") return `colour:${m.attrs.colour}`;
  return m.type;
}

function rank(m: Mark): number {
  return RANK.indexOf(m.type as MarkType);
}

function sortMarks(marks: Mark[]): Mark[] {
  return [...marks].sort((a, b) => rank(a) - rank(b));
}

/** The marks this module knows, from ProseMirror JSON that may carry others or extra attrs. */
function knownMarks(marks: readonly { type: string; attrs?: Record<string, unknown> }[] | undefined): Mark[] {
  const out: Mark[] = [];
  for (const m of marks ?? []) {
    if (m.type === "link") {
      const href = m.attrs?.href;
      if (typeof href === "string") out.push({ type: "link", attrs: { href } });
    } else if (m.type === "colour") {
      const colour = m.attrs?.colour;
      if (typeof colour === "string") out.push({ type: "colour", attrs: { colour } });
    } else if ((RANK as readonly string[]).includes(m.type)) {
      out.push({ type: m.type } as Mark);
    }
  }
  return sortMarks(out);
}

// ---------------------------------------------------------------------------------------------
// Parsing

class Builder {
  nodes: InlineNode[] = [];
  active = new Map<MarkType, Mark>();

  private marks(extra?: Mark): Mark[] | undefined {
    const ms = [...this.active.values()];
    if (extra) ms.push(extra);
    return ms.length ? sortMarks(ms) : undefined;
  }

  text(s: string, extra?: Mark) {
    const marks = this.marks(extra);
    const last = this.nodes[this.nodes.length - 1];
    if (last?.type === "text" && sameMarks(last.marks, marks)) {
      last.text += s;
    } else {
      this.nodes.push(marks ? { type: "text", text: s, marks } : { type: "text", text: s });
    }
  }

  hardBreak(extra?: Mark) {
    const marks = this.marks(extra);
    this.nodes.push(marks ? { type: "hardBreak", marks } : { type: "hardBreak" });
  }

  fieldRef(name: string) {
    const marks = this.marks();
    this.nodes.push(marks ? { type: "fieldRef", attrs: { name }, marks } : { type: "fieldRef", attrs: { name } });
  }

  mention(id: string, label: string | null) {
    const marks = this.marks();
    const attrs = { id, label };
    this.nodes.push(marks ? { type: "mention", attrs, marks } : { type: "mention", attrs });
  }

  placeholderLink(noteType: string, title: string) {
    const marks = this.marks();
    const attrs = { noteType, title };
    this.nodes.push(marks ? { type: "placeholderLink", attrs, marks } : { type: "placeholderLink", attrs });
  }

  toggle(type: MarkType) {
    if (this.active.has(type)) this.active.delete(type);
    else this.active.set(type, { type } as Mark);
  }
}

function sameMarks(a: Mark[] | undefined, b: Mark[] | undefined): boolean {
  const ka = (a ?? []).map(markKey).join("\u0000");
  const kb = (b ?? []).map(markKey).join("\u0000");
  return ka === kb;
}

/** The index of the next unescaped `ch` at or after `from`, or -1. */
function findUnescaped(s: string, ch: string, from: number): number {
  for (let i = from; i < s.length; i++) {
    if (s.charAt(i) === "\\") i++;
    else if (s.charAt(i) === ch) return i;
  }
  return -1;
}

/** Unescape `\x` to `x` for any markup character; other backslashes stay. */
function unescape(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    if (s.charAt(i) === "\\" && i + 1 < s.length && MARKUP_CHARS.has(s.charAt(i + 1))) {
      out += s.charAt(i + 1);
      i++;
    } else {
      out += s.charAt(i);
    }
  }
  return out;
}

/** A mention at `i`: `[[ULID]]` or `[[ULID|label]]`, with its end index. */
function matchMention(s: string, i: number): { id: string; label: string | null; end: number } | null {
  if (!s.startsWith("[[", i)) return null;
  const id = s.slice(i + 2, i + 28);
  if (!isUlid(id)) return null;
  let j = i + 28;
  if (s.startsWith("]]", j)) return { id, label: null, end: j + 2 };
  if (s.charAt(j) !== "|") return null;
  j++;
  const start = j;
  while (j < s.length) {
    if (s.charAt(j) === "\\") {
      j += 2;
    } else if (s.startsWith("]]", j)) {
      return { id, label: unescape(s.slice(start, j)), end: j + 2 };
    } else {
      j++;
    }
  }
  return null;
}

/**
 * A placeholder link at `i`: `[[+type|title]]`, a note of `type` titled `title` that does not
 * exist yet, with its end index. The title runs to the first unescaped `]]` and is not blank.
 */
export function matchPlaceholder(s: string, i: number): { noteType: string; title: string; end: number } | null {
  if (!s.startsWith("[[+", i)) return null;
  const bar = s.indexOf("|", i + 3);
  if (bar < 0) return null;
  const noteType = s.slice(i + 3, bar);
  if (!TYPE_NAME.test(noteType)) return null;
  let j = bar + 1;
  while (j < s.length) {
    if (s.charAt(j) === "\\") {
      j += 2;
    } else if (s.startsWith("]]", j)) {
      const title = unescape(s.slice(bar + 1, j));
      return title.trim() === "" ? null : { noteType, title, end: j + 2 };
    } else {
      j++;
    }
  }
  return null;
}

/** A placeholder link as markup. */
export function placeholderMarkup(noteType: string, title: string): string {
  return `[[+${noteType}|${escapeLabel(title)}]]`;
}

/**
 * `text` with its first placeholder link for a `noteType` titled `title` rewritten as a mention
 * of `id`, leaving every other character as it was; null when it holds no such placeholder.
 * Escaped brackets and code spans hold no placeholder, as in [`parseInline`].
 */
export function replacePlaceholder(text: string, noteType: string, title: string, id: string): string | null {
  for (let i = 0; i < text.length; i++) {
    const c = text.charAt(i);
    if (c === "\\") {
      i++;
    } else if (c === "`") {
      const close = findUnescaped(text, "`", i + 1);
      if (close >= 0) i = close;
    } else if (c === "[") {
      const p = matchPlaceholder(text, i);
      if (p && p.noteType === noteType && p.title === title) return `${text.slice(0, i)}[[${id}]]${text.slice(p.end)}`;
      if (p) i = p.end - 1;
    }
  }
  return null;
}

/**
 * A link opening at `i`: the index of the `]` that ends its label, its href, and the index after
 * the closing `)`. The label may hold mentions and code spans but no unescaped `[` or `]`.
 */
function matchLink(s: string, i: number): { close: number; href: string; end: number } | null {
  let j = i + 1;
  while (j < s.length) {
    const c = s.charAt(j);
    if (c === "\\") {
      j += 2;
      continue;
    }
    if (c === "`") {
      const k = findUnescaped(s, "`", j + 1);
      if (k >= 0) {
        j = k + 1;
        continue;
      }
    }
    if (c === "[") {
      const m = matchMention(s, j) ?? matchPlaceholder(s, j);
      if (!m) return null;
      j = m.end;
      continue;
    }
    if (c === "]") {
      if (s.charAt(j + 1) !== "(") return null;
      const k = findUnescaped(s, ")", j + 2);
      if (k < 0) return null;
      return { close: j, href: unescape(s.slice(j + 2, k)), end: k + 1 };
    }
    j++;
  }
  return null;
}

function runLength(s: string, i: number, ch: string): number {
  let n = 0;
  while (s.charAt(i + n) === ch) n++;
  return n;
}

/** Parse inline markup into ProseMirror inline JSON. Any string parses; unmatched markup is text. */
export function parseInline(s: string, options: ParseOptions = {}): InlineNode[] {
  const b = new Builder();
  let link: { close: number; end: number } | null = null;
  let i = 0;
  while (i < s.length) {
    const c = s.charAt(i);
    if (link && i === link.close) {
      b.active.delete("link");
      i = link.end;
      link = null;
      continue;
    }
    if (c === "\\") {
      if (i + 1 < s.length && MARKUP_CHARS.has(s.charAt(i + 1))) {
        b.text(s.charAt(i + 1));
        i += 2;
      } else {
        b.text("\\");
        i++;
      }
      continue;
    }
    if (c === "\n") {
      b.hardBreak();
      i++;
      continue;
    }
    if (c === "*") {
      let n = runLength(s, i, "*");
      i += n;
      while (n >= 3) {
        b.toggle("bold");
        b.toggle("italic");
        n -= 3;
      }
      if (n === 2) b.toggle("bold");
      if (n === 1) b.toggle("italic");
      continue;
    }
    if (c === "+" || c === "~" || c === "=") {
      let n = runLength(s, i, c);
      i += n;
      for (; n >= 2; n -= 2) b.toggle(TOGGLES[c + c] as MarkType);
      if (n === 1) b.text(c);
      continue;
    }
    if (c === "`") {
      const close = findUnescaped(s, "`", i + 1);
      if (close < 0) {
        b.text("`");
        i++;
        continue;
      }
      const code: Mark = { type: "code" };
      for (let j = i + 1; j < close; j++) {
        if (s.charAt(j) === "\\" && (s.charAt(j + 1) === "`" || s.charAt(j + 1) === "\\")) {
          b.text(s.charAt(j + 1), code);
          j++;
        } else if (s.charAt(j) === "\n") {
          b.hardBreak(code);
        } else {
          b.text(s.charAt(j), code);
        }
      }
      i = close + 1;
      continue;
    }
    if (c === "{") {
      const ref = options.fieldRefs ? FIELD_REF.exec(s.slice(i, i + 80)) : null;
      if (ref) {
        b.fieldRef(ref[1]!);
        i += ref[0].length;
        continue;
      }
      if (s.startsWith("{/}", i) && b.active.has("colour")) {
        b.active.delete("colour");
        i += 3;
        continue;
      }
      const colour = /^\{([a-z]+)\}/.exec(s.slice(i, i + 10))?.[1] ?? "";
      if ((PALETTE as readonly string[]).includes(colour)) {
        b.active.set("colour", { type: "colour", attrs: { colour } });
        i += colour.length + 2;
        continue;
      }
      b.text("{");
      i++;
      continue;
    }
    if (c === "[") {
      const mention = matchMention(s, i);
      if (mention) {
        b.mention(mention.id, mention.label);
        i = mention.end;
        continue;
      }
      const placeholder = matchPlaceholder(s, i);
      if (placeholder) {
        b.placeholderLink(placeholder.noteType, placeholder.title);
        i = placeholder.end;
        continue;
      }
      if (!link) {
        const l = matchLink(s, i);
        if (l) {
          b.active.set("link", { type: "link", attrs: { href: l.href } });
          link = { close: l.close, end: l.end };
          i++;
          continue;
        }
      }
      b.text("[");
      i++;
      continue;
    }
    b.text(c);
    i++;
  }
  return b.nodes;
}

// ---------------------------------------------------------------------------------------------
// Writing

function open(m: Mark): string {
  switch (m.type) {
    case "link":
      return "[";
    case "colour":
      return `{${m.attrs.colour}}`;
    case "bold":
      return "**";
    case "italic":
      return "*";
    case "underline":
      return "++";
    case "strike":
      return "~~";
    case "highlight":
      return "==";
    case "code":
      return "`";
  }
}

function close(m: Mark): string {
  switch (m.type) {
    case "link":
      return `](${m.attrs.href.replace(/[\\)]/g, "\\$&")})`;
    case "colour":
      return "{/}";
    default:
      return open(m);
  }
}

const ALWAYS_ESCAPED = new Set("\\*`{[]");
const PAIRED = new Set("+~=");

/**
 * Plain text outside a code span. `+`, `~` and `=` only mean something doubled, so a single one
 * stays bare unless it touches another of its kind or the edge of the run, where a delimiter
 * could sit beside it.
 */
export function escapeText(t: string): string {
  let out = "";
  for (let k = 0; k < t.length; k++) {
    const ch = t.charAt(k);
    if (ALWAYS_ESCAPED.has(ch)) {
      out += `\\${ch}`;
    } else if (PAIRED.has(ch) && (k === 0 || k === t.length - 1 || t.charAt(k - 1) === ch || t.charAt(k + 1) === ch)) {
      out += `\\${ch}`;
    } else {
      out += ch;
    }
  }
  return out;
}

function escapeCode(t: string): string {
  return t.replace(/[\\`]/g, "\\$&");
}

function escapeLabel(t: string): string {
  return t.replace(/[\\\]|]/g, "\\$&");
}

/** Write ProseMirror inline JSON as inline markup. */
export function serializeInline(
  nodes: readonly { type: string; text?: string; attrs?: Record<string, unknown>; marks?: readonly { type: string; attrs?: Record<string, unknown> }[] }[],
): string {
  let out = "";
  let current: Mark[] = [];
  const step = (want: Mark[]) => {
    const wantKeys = new Set(want.map(markKey));
    const changed = want.length !== current.length || current.some((m) => !wantKeys.has(markKey(m)));
    const keep = new Set(
      current.filter((m) => wantKeys.has(markKey(m)) && !(m.type === "code" && changed)).map(markKey),
    );
    for (const m of [...current].reverse()) if (!keep.has(markKey(m))) out += close(m);
    for (const m of want) if (!keep.has(markKey(m))) out += open(m);
    current = want;
  };
  for (const node of nodes) {
    let marks = knownMarks(node.marks);
    if (node.type === "mention" || node.type === "placeholderLink" || node.type === "fieldRef") marks = marks.filter((m) => m.type !== "code");
    step(marks);
    const inCode = marks.some((m) => m.type === "code");
    if (node.type === "text") {
      const t = node.text ?? "";
      out += inCode ? escapeCode(t) : escapeText(t);
    } else if (node.type === "hardBreak") {
      out += "\n";
    } else if (node.type === "fieldRef") {
      out += `{{${String(node.attrs?.name ?? "")}}}`;
    } else if (node.type === "mention") {
      const id = String(node.attrs?.id ?? "");
      const label = node.attrs?.label;
      out += typeof label === "string" && label !== "" ? `[[${id}|${escapeLabel(label)}]]` : `[[${id}]]`;
    } else if (node.type === "placeholderLink") {
      out += placeholderMarkup(String(node.attrs?.noteType ?? ""), String(node.attrs?.title ?? ""));
    }
  }
  step([]);
  return out;
}

/** The names `{{name}}` references in template text, in order. */
export function fieldRefNames(text: string): string[] {
  return parseInline(text, { fieldRefs: true }).flatMap((n) => (n.type === "fieldRef" ? [n.attrs.name] : []));
}

/**
 * Template text with each `{{name}}` renamed by `rename`, which answers the new name, or null to
 * leave the reference as escaped text. Text with nothing to change comes back as it was.
 */
export function renameFieldRefs(text: string, rename: (name: string) => string | null): string {
  const nodes = parseInline(text, { fieldRefs: true });
  let changed = false;
  const next = nodes.map((n): InlineNode => {
    if (n.type !== "fieldRef") return n;
    const to = rename(n.attrs.name);
    if (to === n.attrs.name) return n;
    changed = true;
    return to === null
      ? { type: "text", text: `{{${n.attrs.name}}}`, ...(n.marks ? { marks: n.marks } : {}) }
      : { ...n, attrs: { name: to } };
  });
  return changed ? serializeInline(next) : text;
}
