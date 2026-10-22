// A template tab's `when` (D14, D42): the condition, in the syntax of a query's `where` clause,
// under which the tab shows for a note. This is the web client's parser, checker and evaluator
// for it, so a tab appears or disappears as fields are edited, before any save. It mirrors the
// server's query evaluator (crates/query), and the cases in crates/query/tests/when_cases.json
// hold in both. A condition reads only the note shown.

import type { Json } from "./api";

export type Expr =
  | { t: "lit"; v: Lit }
  | { t: "name"; name: string }
  | { t: "this" }
  | { t: "field"; base: Expr; name: string }
  | { t: "call"; f: string; args: Expr[] }
  | { t: "not"; e: Expr }
  | { t: "neg"; e: Expr }
  | { t: "bin"; op: BinOp; a: Expr; b: Expr }
  | { t: "closure"; params: string[]; body: Expr };

type Lit = { k: "bool"; v: boolean } | { k: "int"; v: number } | { k: "float"; v: number } | { k: "str"; v: string };

type BinOp = "||" | "&&" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | "%";

/** A condition that does not parse, at a line and column counted from 1. */
export class WhenSyntaxError extends Error {
  constructor(
    readonly line: number,
    readonly column: number,
    readonly reason: string,
  ) {
    super(`${line}:${column}: ${reason}`);
  }
}

/** The query functions, with their argument counts. */
const FUNCTIONS: Record<string, number> = {
  today: 0,
  now: 0,
  days: 1,
  has: 2,
  has_ci: 2,
  under: 2,
  any: 2,
  all: 2,
  linked_from: 2,
  contains: 2,
  empty: 1,
};

/** Words that start a query clause, which a condition cannot use as a name. */
const CLAUSES = new Set(["where", "search", "sort", "group", "select", "limit"]);

/** Longest first, so `<=` is not read as `<` then `=`. */
const SYMBOLS = ["==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-", "*", "/", "%", "(", ")", ",", ".", "|"];

type Tok =
  | { k: "ident"; v: string }
  | { k: "str"; v: string }
  | { k: "int"; v: number }
  | { k: "float"; v: number }
  | { k: "sym"; v: string }
  | { k: "eof" };

interface Pos {
  line: number;
  column: number;
}

function describe(t: Tok): string {
  switch (t.k) {
    case "ident":
    case "sym":
      return `\`${t.v}\``;
    case "str":
      return "a string";
    case "int":
    case "float":
      return "a number";
    case "eof":
      return "the end of the query";
  }
}

function lex(src: string): [Tok, Pos][] {
  const chars = [...src];
  let i = 0;
  let line = 1;
  let column = 1;
  const out: [Tok, Pos][] = [];
  const peek = (n = 0) => chars[i + n];
  const bump = () => {
    const c = chars[i++];
    if (c === "\n") {
      line++;
      column = 1;
    } else column++;
    return c;
  };
  const rest = (n: number) => chars.slice(i, i + n).join("");
  const fail = (at: Pos, m: string) => new WhenSyntaxError(at.line, at.column, m);
  for (;;) {
    while (i < chars.length) {
      if (/\s/.test(peek()!)) bump();
      else if (rest(2) === "//") while (i < chars.length && peek() !== "\n") bump();
      else break;
    }
    const at = { line, column };
    if (i >= chars.length) {
      out.push([{ k: "eof" }, at]);
      return out;
    }
    const c = peek()!;
    if (c === '"') {
      bump();
      let value = "";
      for (;;) {
        const here = { line, column };
        const d = peek();
        if (d === undefined || d === "\n") throw fail(at, "unterminated string");
        if (d === '"') {
          bump();
          break;
        }
        if (d === "\\") {
          bump();
          const e = bump();
          const simple: Record<string, string> = { n: "\n", t: "\t", r: "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'" };
          if (e !== undefined && e in simple) value += simple[e];
          else if (e === "u") {
            const bad = () => fail(here, "invalid unicode escape: expected `\\u{...}`");
            if (bump() !== "{") throw bad();
            let hex = "";
            for (;;) {
              const h = bump();
              if (h === "}") break;
              if (h !== undefined && /[0-9a-fA-F]/.test(h) && hex.length < 6) hex += h;
              else throw bad();
            }
            const code = parseInt(hex, 16);
            if (!hex || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) throw bad();
            value += String.fromCodePoint(code);
          } else throw fail(here, "unknown escape: the escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\u{...}");
          continue;
        }
        if ((d === "{" || d === "}") && peek(1) === d) {
          bump();
          bump();
          value += d;
          continue;
        }
        if (d === "{" && peek(1) !== "}" && peek(1) !== ":") {
          throw fail(here, "a query string cannot interpolate: write `{{` for a literal brace");
        }
        value += bump();
      }
      out.push([{ k: "str", v: value }, at]);
    } else if (/[0-9]/.test(c)) {
      let text = "";
      let float = false;
      while (i < chars.length) {
        const d = peek()!;
        if (/[0-9]/.test(d)) text += d;
        else if (d === "_") {
          // A digit separator.
        } else if (d === "." && !float && /[0-9]/.test(peek(1) ?? "")) {
          float = true;
          text += d;
        } else break;
        bump();
      }
      const v = Number(text);
      if (!float && !Number.isSafeInteger(v)) throw fail(at, `\`${text}\` is too large for an int`);
      out.push([{ k: float ? "float" : "int", v }, at]);
    } else if (/[\p{L}_]/u.test(c)) {
      let word = "";
      while (i < chars.length && /[\p{L}\p{N}_]/u.test(peek()!)) word += bump();
      out.push([{ k: "ident", v: word }, at]);
    } else {
      const s = SYMBOLS.find((s) => rest(s.length) === s);
      if (!s) throw fail(at, `unexpected character \`${c}\``);
      for (let n = 0; n < s.length; n++) bump();
      out.push([{ k: "sym", v: s }, at]);
    }
  }
}

const PREC: Record<BinOp, number> = {
  "||": 1,
  "&&": 2,
  "==": 3,
  "!=": 3,
  "<": 3,
  "<=": 3,
  ">": 3,
  ">=": 3,
  "+": 4,
  "-": 4,
  "*": 5,
  "/": 5,
  "%": 5,
};

/** Parse a condition. Throws [`WhenSyntaxError`] where it does not parse. */
export function parseWhen(text: string): Expr {
  const toks = lex(text);
  let i = 0;
  const tok = () => toks[i]![0];
  const pos = () => toks[i]![1];
  const bump = () => {
    const t = tok();
    if (i + 1 < toks.length) i++;
    return t;
  };
  const isSym = (s: string) => tok().k === "sym" && (tok() as { v: string }).v === s;
  const eatSym = (s: string) => {
    const found = isSym(s);
    if (found) bump();
    return found;
  };
  const fail = (at: Pos, m: string) => new WhenSyntaxError(at.line, at.column, m);
  const unexpected = (wanted: string) => fail(pos(), `expected ${wanted}, found ${describe(tok())}`);
  const expectSym = (s: string) => {
    if (!eatSym(s)) throw unexpected(`\`${s}\``);
  };
  const name = (what: string): string => {
    const t = tok();
    if (t.k === "ident" && !CLAUSES.has(t.v)) {
      bump();
      return t.v;
    }
    throw unexpected(what);
  };

  function expr(): Expr {
    return binary(1);
  }

  function binary(min: number): Expr {
    let lhs = unary();
    for (;;) {
      const t = tok();
      if (t.k !== "sym" || !(t.v in PREC)) return lhs;
      const op = t.v as BinOp;
      if (PREC[op] < min) return lhs;
      bump();
      const rhs = binary(PREC[op] + 1);
      lhs = { t: "bin", op, a: lhs, b: rhs };
    }
  }

  function unary(): Expr {
    if (eatSym("!")) return { t: "not", e: unary() };
    if (eatSym("-")) return { t: "neg", e: unary() };
    return postfix();
  }

  function postfix(): Expr {
    let e = primary();
    for (;;) {
      if (eatSym(".")) {
        const n = name("a field name after `.`");
        if (isSym("(")) throw fail(pos(), "methods cannot be called in a query: only the query functions can");
        e = { t: "field", base: e, name: n };
      } else if (isSym("(")) throw fail(pos(), "only a query function can be called");
      else return e;
    }
  }

  function primary(): Expr {
    const at = pos();
    const t = tok();
    if ((t.k === "ident" && CLAUSES.has(t.v)) || t.k === "eof") throw unexpected("an expression");
    if (t.k === "sym" && t.v !== "(" && t.v !== "|") throw unexpected("an expression");
    bump();
    switch (t.k) {
      case "int":
        return { t: "lit", v: { k: "int", v: t.v } };
      case "float":
        return { t: "lit", v: { k: "float", v: t.v } };
      case "str":
        return { t: "lit", v: { k: "str", v: t.v } };
      case "sym":
        if (t.v === "(") {
          const e = expr();
          expectSym(")");
          return e;
        }
        throw fail(at, "a closure can only be the last argument of `any` or `all`");
      case "ident":
        if (t.v === "true" || t.v === "false") return { t: "lit", v: { k: "bool", v: t.v === "true" } };
        if (t.v === "this") return { t: "this" };
        if (isSym("(")) return call(t.v, at);
        return { t: "name", name: t.v };
    }
  }

  function call(f: string, at: Pos): Expr {
    const arity = FUNCTIONS[f];
    if (arity === undefined) {
      throw fail(at, `unknown function \`${f}\`: the query functions are ${Object.keys(FUNCTIONS).join(", ")}`);
    }
    expectSym("(");
    const closureLast = f === "any" || f === "all";
    const args: Expr[] = [];
    if (!isSym(")")) {
      for (;;) {
        if (closureLast && args.length + 1 === arity) args.push(closure(f));
        else args.push(expr());
        if (!eatSym(",")) break;
      }
    }
    if (args.length !== arity) {
      throw fail(at, `\`${f}\` takes ${arity} argument${arity === 1 ? "" : "s"}, found ${args.length}`);
    }
    expectSym(")");
    return { t: "call", f, args };
  }

  function closure(f: string): Expr {
    if (!eatSym("|")) throw unexpected(`a closure such as \`|x| x.done\` as the last argument of \`${f}\``);
    const params = [name("a closure parameter")];
    while (eatSym(",")) params.push(name("a closure parameter"));
    expectSym("|");
    if (params.length !== 1) throw unexpected("one closure parameter");
    return { t: "closure", params, body: expr() };
  }

  const e = expr();
  if (tok().k !== "eof") throw unexpected("an operator or the end of the expression");
  return e;
}

/** What a condition needs to know of a field. */
export interface WhenField {
  name: string;
  kind: string;
  of?: string | null;
  items?: string | null;
  values?: string | null;
}

/** Fields every note has, besides its type's. */
const BUILTINS = new Set(["id", "type", "title", "created", "updated", "archived"]);

/**
 * What is wrong with a condition on a type with `fields`, as the server reports it when the type
 * is saved, or null when nothing is: it must parse, name only fields the note has, and read no
 * other note.
 */
export function whenProblem(text: string, fields: WhenField[]): string | null {
  let e: Expr;
  try {
    e = parseWhen(text);
  } catch (err) {
    if (err instanceof WhenSyntaxError) return `does not parse: ${err.message}`;
    throw err;
  }
  const kinds = new Map(fields.map((f) => [f.name, f.kind]));
  const known = (n: string) => (BUILTINS.has(n) || kinds.has(n) ? null : `\`${n}\` is not a field of the type`);
  const check = (e: Expr, bound: string[]): string | null => {
    switch (e.t) {
      case "lit":
      case "this":
        return null;
      case "name":
        return bound.includes(e.name) ? null : known(e.name);
      case "field": {
        const b = e.base;
        let name: string;
        if (b.t === "this") return known(e.name);
        if (b.t === "name" && !bound.includes(b.name)) name = b.name;
        else if (b.t === "field" && b.base.t === "this") name = b.name;
        else return `\`.${e.name}\` reads another note, which a tab condition cannot`;
        const k = known(name);
        if (k) return k;
        const kind = kinds.get(name);
        if (kind === "date_range" || kind === "location" || kind === "dict") return null;
        if (kind === "reference") return `\`${name}.${e.name}\` reads another note, which a tab condition cannot`;
        return `\`${name}\` has no members: \`.\` reads a date range's start or end`;
      }
      case "call":
        if (e.f === "linked_from") return "linked_from() reads other notes, which a tab condition cannot";
        for (const a of e.args) {
          const p = check(a, bound);
          if (p) return p;
        }
        return null;
      case "not":
      case "neg":
        return check(e.e, bound);
      case "bin":
        return check(e.a, bound) ?? check(e.b, bound);
      case "closure":
        return check(e.body, [...bound, ...e.params]);
    }
  };
  return check(e, []);
}

// Values while evaluating. Dates are days since 1970-01-01, datetimes and durations seconds.
type V =
  | { k: "null" }
  | { k: "bool"; v: boolean }
  | { k: "int"; v: number }
  | { k: "float"; v: number }
  | { k: "str"; v: string }
  | { k: "date"; v: number }
  | { k: "datetime"; v: number }
  | { k: "duration"; v: number }
  | { k: "ref"; v: string }
  | { k: "list"; v: V[] }
  | { k: "record"; v: [string, V][] };

const NULL: V = { k: "null" };

class EvalError extends Error {}

const KIND: Record<V["k"], string> = {
  null: "nothing",
  bool: "a bool",
  int: "an int",
  float: "a float",
  str: "a string",
  date: "a date",
  datetime: "a datetime",
  duration: "a duration",
  ref: "a note",
  list: "a list",
  record: "a record",
};

function truth(v: V): boolean {
  if (v.k === "bool") return v.v;
  if (v.k === "null") return false;
  throw new EvalError(`expected a bool, found ${KIND[v.k]}`);
}

function timeOf(v: V): number | null {
  if (v.k === "date") return v.v * 86_400;
  if (v.k === "datetime") return v.v;
  return null;
}

function numOf(v: V): number | null {
  return v.k === "int" || v.k === "float" ? v.v : null;
}

function idOf(v: V): string | null {
  return v.k === "str" || v.k === "ref" ? v.v : null;
}

function digits(s: string | undefined): number | null {
  return s !== undefined && s.length > 0 && /^[0-9]+$/.test(s) ? Number(s) : null;
}

function daysFromCivil(y: number, m: number, d: number): number {
  const yy = m <= 2 ? y - 1 : y;
  const era = Math.floor(yy / 400);
  const yoe = yy - era * 400;
  const mp = (m + 9) % 12;
  const doy = Math.floor((153 * mp + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146_097 + doe - 719_468;
}

/** Days since 1970-01-01 of `YYYY-MM-DD`. */
function parseDate(s: string): number | null {
  if (s.length !== 10 || s[4] !== "-" || s[7] !== "-") return null;
  const y = digits(s.slice(0, 4));
  const m = digits(s.slice(5, 7));
  const d = digits(s.slice(8, 10));
  if (y === null || m === null || d === null || m < 1 || m > 12 || d < 1 || d > 31) return null;
  return daysFromCivil(y, m, d);
}

/** Seconds since the Unix epoch of `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`. */
function parseDatetime(s: string): number | null {
  if (s.length < 20 || s[10] !== "T") return null;
  const days = parseDate(s.slice(0, 10));
  if (days === null) return null;
  const t = s.slice(11);
  const h = digits(t.slice(0, 2));
  const mi = digits(t.slice(3, 5));
  const sec = digits(t.slice(6, 8));
  if (h === null || mi === null || sec === null) return null;
  let rest = t.slice(8);
  let frac = 0;
  if (rest.startsWith(".")) {
    const m = /^[0-9]*/.exec(rest.slice(1))![0];
    frac = Number(`0.${m}`);
    rest = rest.slice(1 + m.length);
  }
  let offset: number;
  if (rest[0] === "Z") offset = 0;
  else if (rest[0] === "+" || rest[0] === "-") {
    const oh = digits(rest.slice(1, 3));
    const om = digits(rest.slice(4, 6));
    if (oh === null || om === null) return null;
    offset = (oh * 3600 + om * 60) * (rest[0] === "+" ? 1 : -1);
  } else return null;
  return days * 86_400 + h * 3600 + mi * 60 + sec - offset + frac;
}

function strTime(s: string): number | null {
  const d = parseDate(s);
  return d !== null ? d * 86_400 : parseDatetime(s);
}

function times(a: V, b: V): [number, number] | null {
  const x = timeOf(a);
  const y = timeOf(b);
  if (x !== null && y !== null) return [x, y];
  if (x !== null) {
    const s = idOf(b);
    const t = s === null ? null : strTime(s);
    return t === null ? null : [x, t];
  }
  if (y !== null) {
    const s = idOf(a);
    const t = s === null ? null : strTime(s);
    return t === null ? null : [t, y];
  }
  return null;
}

function equals(a: V, b: V): boolean {
  const t = times(a, b);
  if (t) return t[0] === t[1];
  if (a.k === "null" && b.k === "null") return true;
  if (a.k === "null" || b.k === "null") return false;
  if (a.k === "bool" && b.k === "bool") return a.v === b.v;
  if (a.k === "duration" && b.k === "duration") return a.v === b.v;
  if (a.k === "list" && b.k === "list") {
    if (a.v.length !== b.v.length) return false;
    let all = true;
    a.v.forEach((x, i) => (all = equals(x, b.v[i]!) && all));
    return all;
  }
  if (a.k === "record" && b.k === "record") {
    if (a.v.length !== b.v.length) return false;
    let all = true;
    a.v.forEach(([k, x], i) => {
      const [l, y] = b.v[i]!;
      all = equals(x, y) && all && k === l;
    });
    return all;
  }
  const [x, y] = [numOf(a), numOf(b)];
  if (x !== null && y !== null) return x === y;
  const [p, q] = [idOf(a), idOf(b)];
  if (p !== null && q !== null) return p === q;
  throw new EvalError(`cannot compare ${KIND[a.k]} with ${KIND[b.k]}`);
}

/** -1, 0 or 1, or null when either side is missing. */
function compare(a: V, b: V): number | null {
  if (a.k === "null" || b.k === "null") return null;
  const cmp = (x: number, y: number) => (Number.isNaN(x) || Number.isNaN(y) ? null : x < y ? -1 : x > y ? 1 : 0);
  const t = times(a, b);
  if (t) return cmp(t[0], t[1]);
  if (a.k === "duration" && b.k === "duration") return cmp(a.v, b.v);
  if (a.k === "str" && b.k === "str") return a.v < b.v ? -1 : a.v > b.v ? 1 : 0;
  const [x, y] = [numOf(a), numOf(b)];
  if (x !== null && y !== null) return cmp(x, y);
  throw new EvalError(`cannot order ${KIND[a.k]} against ${KIND[b.k]}`);
}

function int(n: number): V {
  if (!Number.isSafeInteger(n)) throw new EvalError("integer overflow");
  return { k: "int", v: n };
}

function addSub(a: V, b: V, add: boolean): V {
  const sign = add ? 1 : -1;
  if (a.k === "null" || b.k === "null") return NULL;
  if (a.k === "int" && b.k === "int") return int(a.v + sign * b.v);
  if (a.k === "date" && b.k === "duration") {
    const secs = a.v * 86_400 + sign * b.v;
    return b.v % 86_400 === 0 ? { k: "date", v: Math.trunc(secs / 86_400) } : { k: "datetime", v: secs };
  }
  if (a.k === "datetime" && b.k === "duration") return { k: "datetime", v: a.v + sign * b.v };
  if (a.k === "duration" && b.k === "duration") return { k: "duration", v: a.v + sign * b.v };
  if (add && a.k === "duration" && b.k === "date") return addSub(b, a, true);
  if (add && a.k === "duration" && b.k === "datetime") return { k: "datetime", v: b.v + a.v };
  if (!add && (a.k === "date" || a.k === "datetime") && (b.k === "date" || b.k === "datetime")) {
    return { k: "duration", v: timeOf(a)! - timeOf(b)! };
  }
  if (add && a.k === "str" && b.k === "str") return { k: "str", v: a.v + b.v };
  const [x, y] = [numOf(a), numOf(b)];
  if (x !== null && y !== null) return { k: "float", v: x + sign * y };
  throw new EvalError(`cannot apply \`${add ? "+" : "-"}\` to ${KIND[a.k]} and ${KIND[b.k]}`);
}

function mulDiv(a: V, b: V, op: "*" | "/" | "%"): V {
  if (a.k === "null" || b.k === "null") return NULL;
  if (a.k === "int" && b.k === "int") {
    if (op === "*") return int(a.v * b.v);
    if (b.v === 0) throw new EvalError("division by zero");
    return op === "/" ? int(Math.trunc(a.v / b.v)) : int(a.v % b.v);
  }
  if (op === "*" && a.k === "duration" && numOf(b) !== null) return { k: "duration", v: a.v * numOf(b)! };
  if (op === "*" && b.k === "duration" && numOf(a) !== null) return { k: "duration", v: b.v * numOf(a)! };
  if (op === "/" && a.k === "duration" && numOf(b) !== null) return { k: "duration", v: a.v / numOf(b)! };
  const [x, y] = [numOf(a), numOf(b)];
  if (x !== null && y !== null) return { k: "float", v: op === "*" ? x * y : op === "/" ? x / y : x % y };
  throw new EvalError(`cannot apply \`${op}\` to ${KIND[a.k]} and ${KIND[b.k]}`);
}

/** A JSON value with no field kind to go by. */
function plain(raw: Json): V {
  if (raw === null) return NULL;
  if (typeof raw === "boolean") return { k: "bool", v: raw };
  if (typeof raw === "number") return Number.isInteger(raw) ? { k: "int", v: raw } : { k: "float", v: raw };
  if (typeof raw === "string") return { k: "str", v: raw };
  if (Array.isArray(raw)) return { k: "list", v: raw.map(plain) };
  return { k: "record", v: Object.entries(raw).map(([k, v]) => [k, plain(v as Json)]) };
}

/** A stored field value, typed by its field's kind. */
function fromJson(raw: Json, kind: string, f?: WhenField): V {
  if (Array.isArray(raw)) return { k: "list", v: raw.map((i) => fromJson(i, kind, f)) };
  if (raw === null) return NULL;
  if (kind === "date" && typeof raw === "string") {
    const d = parseDate(raw);
    return d === null ? { k: "str", v: raw } : { k: "date", v: d };
  }
  if (kind === "datetime" && typeof raw === "string") {
    const t = parseDatetime(raw);
    return t === null ? { k: "str", v: raw } : { k: "datetime", v: t };
  }
  if (kind === "date_range" && typeof raw === "object") {
    return {
      k: "record",
      v: Object.entries(raw).map(([k, v]) => {
        // Either form reads by its shape: a date is ten characters.
        const datetime = f?.of === "datetime" || (f?.of === "date_or_datetime" && typeof v === "string" && v.length !== 10);
        return [k, fromJson(v as Json, datetime ? "datetime" : "date")];
      }),
    };
  }
  if (kind === "list") return fromJson(raw, f?.items === "taxonomy" ? "string" : (f?.items ?? "string"));
  if (kind === "dict") return fromJson(raw, f?.values ?? "string");
  if (kind === "reference" && typeof raw === "string") return { k: "ref", v: raw };
  if (kind === "float" && typeof raw === "number") return { k: "float", v: raw };
  return plain(raw);
}

/** The note a condition reads. */
export interface WhenNote {
  id: string;
  type: string;
  /** The note's values by field name: `title`, `created`, `updated`, `archived` and its type's fields. */
  fields: Record<string, Json | undefined>;
  /** The type's fields. */
  defs: WhenField[];
}

export interface WhenClock {
  /** Seconds since the Unix epoch. */
  now: number;
  /** Local time's offset from UTC in seconds, for `today()`. */
  utcOffset: number;
}

/** The clock of this browser, now. */
export function localClock(): WhenClock {
  const now = Date.now() / 1000;
  return { now, utcOffset: -new Date().getTimezoneOffset() * 60 };
}

function evaluate(e: Expr, note: WhenNote, clock: WhenClock): boolean {
  const env: [string, V][] = [];
  const defs = new Map(note.defs.map((f) => [f.name, f]));
  const field = (name: string): V => {
    const raw = note.fields[name];
    const time = (s: Json | undefined): V => {
      const t = typeof s === "string" ? parseDatetime(s) : null;
      return t === null ? NULL : { k: "datetime", v: t };
    };
    switch (name) {
      case "id":
        return { k: "ref", v: note.id };
      case "type":
        return { k: "str", v: note.type };
      case "title":
        return { k: "str", v: typeof raw === "string" ? raw : "" };
      case "created":
      case "updated":
        return time(raw);
      case "archived":
        return { k: "bool", v: raw === true };
    }
    const def = defs.get(name);
    return def && raw !== undefined ? fromJson(raw, def.kind, def) : NULL;
  };
  const member = (base: V, name: string): V => {
    if (base.k === "null") return NULL;
    // Only the note shown can be read: a reference to another note reads as missing.
    if (base.k === "ref") return base.v === note.id ? field(name) : NULL;
    if (base.k === "record") return base.v.find(([k]) => k === name)?.[1] ?? NULL;
    if (base.k === "list") throw new EvalError(`\`.${name}\` follows a single reference, but this is a list: use any() or all()`);
    throw new EvalError(`\`.${name}\` cannot read a member of ${KIND[base.k]}`);
  };
  const isTrue = (e: Expr) => truth(ev(e));
  const ev = (e: Expr): V => {
    switch (e.t) {
      case "lit":
        return e.v;
      case "this":
        return { k: "ref", v: note.id };
      case "name": {
        for (let i = env.length - 1; i >= 0; i--) if (env[i]![0] === e.name) return env[i]![1];
        return field(e.name);
      }
      case "field":
        return member(ev(e.base), e.name);
      case "not":
        return { k: "bool", v: !isTrue(e.e) };
      case "neg": {
        const v = ev(e.e);
        if (v.k === "int") return int(-v.v);
        if (v.k === "float" || v.k === "duration") return { k: v.k, v: -v.v };
        if (v.k === "null") return NULL;
        throw new EvalError(`cannot negate ${KIND[v.k]}`);
      }
      case "bin": {
        if (e.op === "&&") return { k: "bool", v: isTrue(e.a) && isTrue(e.b) };
        if (e.op === "||") return { k: "bool", v: isTrue(e.a) || isTrue(e.b) };
        const a = ev(e.a);
        const b = ev(e.b);
        const ord = (want: (o: number) => boolean): V => {
          const o = compare(a, b);
          return { k: "bool", v: o !== null && want(o) };
        };
        switch (e.op) {
          case "==":
            return { k: "bool", v: equals(a, b) };
          case "!=":
            return { k: "bool", v: !equals(a, b) };
          case "<":
            return ord((o) => o < 0);
          case "<=":
            return ord((o) => o <= 0);
          case ">":
            return ord((o) => o > 0);
          case ">=":
            return ord((o) => o >= 0);
          case "+":
            return addSub(a, b, true);
          case "-":
            return addSub(a, b, false);
          default:
            return mulDiv(a, b, e.op);
        }
      }
      case "closure":
        throw new EvalError("a closure is only an argument of any() or all()");
      case "call":
        return call(e.f, e.args);
    }
  };
  const call = (f: string, args: Expr[]): V => {
    switch (f) {
      case "now":
        return { k: "datetime", v: clock.now };
      case "today":
        return { k: "date", v: Math.floor((Math.trunc(clock.now) + clock.utcOffset) / 86_400) };
      case "days": {
        const v = ev(args[0]!);
        if (v.k === "int" || v.k === "float") return { k: "duration", v: v.v * 86_400 };
        throw new EvalError(`days() takes a number, found ${KIND[v.k]}`);
      }
      case "has":
      case "has_ci": {
        const list = ev(args[0]!);
        const item = ev(args[1]!);
        // has_ci compares text ignoring case, and every other value as has does.
        const same = (a: V, b: V) =>
          f === "has_ci" && a.k === "str" && b.k === "str" ? a.v.toLowerCase() === b.v.toLowerCase() : equals(a, b);
        if (list.k === "null") return { k: "bool", v: false };
        if (list.k === "list") return { k: "bool", v: list.v.some((i) => same(i, item)) };
        return { k: "bool", v: same(list, item) };
      }
      case "under": {
        const held = ev(args[0]!);
        const term = ev(args[1]!);
        if (term.k !== "str") throw new EvalError(`under() takes a term name, found ${KIND[term.k]}`);
        const below = (v: V) => v.k === "str" && (v.v === term.v || v.v.startsWith(`${term.v}/`));
        return { k: "bool", v: held.k === "list" ? held.v.some(below) : below(held) };
      }
      case "contains": {
        const held = ev(args[0]!);
        const part = ev(args[1]!);
        if (part.k !== "str") throw new EvalError(`contains() looks for text, found ${KIND[part.k]}`);
        const p = part.v.toLowerCase();
        const within = (v: V) => {
          if (v.k === "null") return false;
          if (v.k === "str") return v.v.toLowerCase().includes(p);
          throw new EvalError(`contains() reads text, found ${KIND[v.k]}`);
        };
        return { k: "bool", v: held.k === "list" ? held.v.some(within) : within(held) };
      }
      case "empty": {
        const v = ev(args[0]!);
        const empty =
          v.k === "null" ||
          (v.k === "str" && v.v.trim() === "") ||
          (v.k === "list" && v.v.length === 0) ||
          (v.k === "record" && v.v.every(([, x]) => x.k === "null"));
        return { k: "bool", v: empty };
      }
      case "any":
      case "all": {
        const held = ev(args[0]!);
        const items = held.k === "null" ? [] : held.k === "list" ? held.v : [held];
        const c = args[1]!;
        if (c.t !== "closure") throw new EvalError(`${f}() takes a closure as its last argument`);
        const wantAny = f === "any";
        for (const item of items) {
          env.push([c.params[0]!, item]);
          try {
            if (isTrue(c.body) === wantAny) return { k: "bool", v: wantAny };
          } finally {
            env.pop();
          }
        }
        return { k: "bool", v: !wantAny };
      }
      default:
        // linked_from reads other notes, which a condition cannot.
        throw new EvalError(`${f}() cannot be used in a tab condition`);
    }
  };
  return isTrue(e);
}

/**
 * Whether `when` holds for `note`. A condition that does not parse or cannot be evaluated is
 * false, as a query's `where` leaves out a note it cannot evaluate.
 */
export function whenHolds(when: string, note: WhenNote, clock: WhenClock = localClock()): boolean {
  try {
    return evaluate(parseWhen(when), note, clock);
  } catch (e) {
    if (e instanceof WhenSyntaxError || e instanceof EvalError) return false;
    throw e;
  }
}

/**
 * The tabs that show for a note: those with no `when` and those whose `when` holds. When none
 * holds, the first tab shows, so a note never renders empty.
 */
export function visibleTabs<T extends { when?: string | null }>(
  tabs: T[],
  note: WhenNote,
  clock: WhenClock = localClock(),
): { tab: T; index: number }[] {
  const shown = tabs.map((tab, index) => ({ tab, index })).filter(({ tab }) => !tab.when || whenHolds(tab.when, note, clock));
  return shown.length || !tabs.length ? shown : [{ tab: tabs[0]!, index: 0 }];
}

/**
 * The tabs a template shows for a note, as [`visibleTabs`] says; a card shows only the first of
 * them.
 */
export function shownTabs<T extends { when?: string | null }>(tabs: T[], note: WhenNote, card: boolean, clock?: WhenClock): T[] {
  const shown = visibleTabs(tabs, note, clock).map((t) => t.tab);
  return card ? shown.slice(0, 1) : shown;
}
