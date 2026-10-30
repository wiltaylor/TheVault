// Check that WCL files stay inside TheVault's data-only subset (D20).
//
// Usage: node data-only.mjs <file.wcl>...
//
// This parses the grammar in docs/vault-format.md ("The data-only subset")
// and nothing else. Any expression, interpolation, import, declaration,
// decorator, `let`, table or connection is reported with its line. It is a
// reference check for the example files; the server's own check lives in
// the `store` crate.

import { readFileSync } from "node:fs";

const RESERVED_ITEMS = new Set([
  "import", "let", "type", "interface", "union", "symbol_set",
  "namespace", "use", "connection", "fn",
]);

class DataOnlyError extends Error {
  constructor(line, message) {
    super(message);
    this.line = line;
  }
}

function lex(src) {
  const tokens = [];
  let i = 0;
  let line = 1;
  const fail = (msg) => { throw new DataOnlyError(line, msg); };
  const push = (kind, value, extra = {}) => tokens.push({ kind, value, line, ...extra });

  while (i < src.length) {
    const c = src[i];
    if (c === "\n") { push("nl", "\n"); line++; i++; continue; }
    if (c === " " || c === "\t" || c === "\r") { i++; continue; }
    if (c === "#" || (c === "/" && src[i + 1] === "/")) {
      while (i < src.length && src[i] !== "\n") i++;
      continue;
    }
    if (c === "$") fail("interpolation (`$\"...\"` or `$<<`) is an expression");
    if (c === '"') {
      let j = i + 1;
      let value = "";
      while (true) {
        if (j >= src.length || src[j] === "\n") fail("unterminated string");
        const d = src[j];
        if (d === '"') break;
        if (d === "\\") {
          const e = src[j + 1];
          const map = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r" };
          if (!(e in map)) fail(`escape \\${e} is not allowed in a string`);
          value += map[e];
          j += 2;
          continue;
        }
        value += d;
        j++;
      }
      push("string", value);
      i = j + 1;
      continue;
    }
    if (c === "<" && src[i + 1] === "<") {
      const m = /^<<('?)([A-Za-z_][A-Za-z0-9_]*)\1[ \t]*\n/.exec(src.slice(i));
      if (!m) fail("malformed heredoc opener");
      const tag = m[2];
      const startLine = line;
      let j = i + m[0].length;
      line++;
      while (true) {
        if (j >= src.length) throw new DataOnlyError(startLine, `heredoc ${tag} is never closed`);
        const end = src.indexOf("\n", j);
        const text = src.slice(j, end === -1 ? src.length : end);
        j = end === -1 ? src.length : end;
        if (text.trim() === tag) break;
        if (end === -1) throw new DataOnlyError(startLine, `heredoc ${tag} is never closed`);
        j++;
        line++;
      }
      push("heredoc", tag, { line: startLine });
      i = j;
      continue;
    }
    if (/[0-9]/.test(c) || (c === "-" && /[0-9]/.test(src[i + 1] ?? ""))) {
      const m = /^-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?/.exec(src.slice(i));
      const after = src[i + m[0].length] ?? "";
      if (/[A-Za-z0-9_.]/.test(after)) {
        fail(`number ${m[0]}${after}... has a suffix, radix prefix or unit; write a plain decimal`);
      }
      push("number", m[0]);
      i += m[0].length;
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_\-/]*/.exec(src.slice(i));
      let word = m[0];
      // A trailing `-` or `/` belongs to an operator, not to the word.
      while (/[-/]$/.test(word)) word = word.slice(0, -1);
      if (src[i + word.length] === '"') fail(`string prefix ${word}"..." is not allowed; strings are plain utf8`);
      push("word", word);
      i += word.length;
      continue;
    }
    if ("={}[],:".includes(c)) { push(c, c); i++; continue; }
    if (c === "@") fail("decorators are not allowed in vault files");
    fail(`\`${c}\` is an operator or expression syntax, which vault files do not allow`);
  }
  push("eof", "");
  return tokens;
}

function parse(tokens) {
  let p = 0;
  const peek = () => tokens[p];
  const next = () => tokens[p++];
  const fail = (tok, msg) => { throw new DataOnlyError(tok.line, msg); };
  const skipNl = () => { while (peek().kind === "nl") p++; };
  const isIdent = (w) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(w);

  function value() {
    skipNl();
    const t = next();
    switch (t.kind) {
      case "string":
      case "heredoc":
      case "number":
        return;
      case "word":
        if (t.value === "true" || t.value === "false") return;
        if (t.value === "none") fail(t, "`none` is not allowed; omit the field instead");
        return fail(t, `\`${t.value}\` is a reference or keyword, which is an expression`);
      case "[":
        skipNl();
        if (peek().kind === "]") { next(); return; }
        while (true) {
          value();
          skipNl();
          const sep = next();
          if (sep.kind === "]") return;
          if (sep.kind !== ",") fail(sep, "expected `,` or `]` in a list");
          skipNl();
          if (peek().kind === "]") { next(); return; }
        }
      case "{":
        skipNl();
        if (peek().kind === "}") { next(); return; }
        while (true) {
          const key = next();
          if (key.kind !== "word" || !isIdent(key.value)) fail(key, "expected a record key");
          const colon = next();
          if (colon.kind !== ":") fail(colon, "expected `:` after a record key");
          value();
          skipNl();
          const sep = next();
          if (sep.kind === "}") return;
          if (sep.kind !== ",") fail(sep, "expected `,` or `}` in a record");
          skipNl();
          if (peek().kind === "}") { next(); return; }
        }
      default:
        return fail(t, `expected a literal value, found \`${t.value}\``);
    }
  }

  function items(closing) {
    while (true) {
      skipNl();
      const t = peek();
      if (t.kind === closing) { next(); return; }
      if (t.kind !== "word") fail(t, `expected a field or block, found \`${t.value}\``);
      next();
      const after = peek();
      // A field may be named after a keyword, as a trigger's `type = "email"` is.
      if (RESERVED_ITEMS.has(t.value) && after.kind !== "=") {
        fail(t, `\`${t.value}\` items are not allowed in vault files`);
      }
      if (after.kind === "=") {
        if (!isIdent(t.value)) fail(t, `\`${t.value}\` is not a valid field name`);
        next();
        value();
      } else if (after.kind === ":") {
        fail(t, "tables are not allowed in vault files");
      } else {
        if (!isIdent(t.value)) fail(t, `\`${t.value}\` is not a valid block kind`);
        while (peek().kind === "string" || peek().kind === "word") next();
        const end = peek();
        if (end.kind === "{") {
          next();
          items("}");
        } else if (end.kind !== "nl" && end.kind !== "eof") {
          fail(end, `unexpected \`${end.value}\` after block \`${t.value}\``);
        }
      }
    }
  }

  items("eof");
}

let failed = false;
for (const file of process.argv.slice(2)) {
  try {
    parse(lex(readFileSync(file, "utf8")));
    console.log(`ok    data-only   ${file}`);
  } catch (e) {
    if (!(e instanceof DataOnlyError)) throw e;
    console.log(`FAIL  data-only   ${file}:${e.line}: ${e.message}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
