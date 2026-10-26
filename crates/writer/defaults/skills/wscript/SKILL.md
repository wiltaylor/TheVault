---
name: wscript
description: Write and fix wscript, the vault's typed scripting language. Use before calling run_script, before proposing a script tool (tools/<name>.wcl and its script), and when writing or debugging a type handler, a scheduled task or any scripts/<name>.ws file, including bulk changes to many notes.
---

# wscript

wscript is a small, statically typed language with Rust-like syntax. The vault runs it for
`run_script`, script tools, type handlers and scheduled tasks. `run_saved_script` calls a
function of a saved script, `scripts/<name>.ws`, by name, and `run_task` starts a scheduled task
now (`references/run-script.md`). A script compiles before it runs:
a type error, an unknown function or a module the run lacks is a compile error with a message.
Read the error, fix the code and run it again.

## The rules that trip people up

- Annotate every function signature: `fn total(xs: List[int]) -> int { ... }`. Nothing else
  needs annotations.
- No implicit conversion: `1 + 2.0` does not compile. Use `int(x)` and `float(x)`.
- No truthiness: an `if` or `while` condition must be a `bool`.
- A block's last expression is its value. A function with no return type returns unit.
- An `if` without `else` must end in unit: end a last call that returns a value with `;` or bind it,
  as in `if !dry_run { let saved = vault::patch(id, fields, "")? }`.
- Statements end at a newline. Semicolons are allowed and never needed.
- Every string literal interpolates `{expr}`. Write a literal brace as `{{` or `}}`. Build JSON
  with `json::to_string`, not by hand.
- `xs[i]` and `m[k]` fault when the item is missing. `.get(...)` returns an `Option` instead.
- Functions that can fail return `Result[T, string]`. Use `?` to pass the error up.
- Notes travel as `Value`, a dynamic JSON-like enum. Read one with `.get("key")`, `.at(i)`,
  `.as_string()`, `.as_int()` and the like, which all return `Option`. Build one with
  `Value::Map(#{ ... })`, `Value::String(...)` and the other variants.
- `use` each host module the script calls: `use vault`, `use http`, `use json`.
- Every argument the host passes to an entry function arrives as a `Value`: `run_script`'s and
  `run_saved_script`'s `args`, a tool's params map, a handler's note. Declare those parameters
  `Value`.

## Where to read more

Read the reference that matches the task with `read_skill_file`:

| File | Read it for |
|------|-------------|
| `references/language.md` | Syntax, types, functions, control flow, `match`, `Option`, `Result`, strings, lists, maps, structs, enums, and the standard modules `value`, `json`, `math` and `regex` |
| `references/vault-api.md` | The host modules `vault`, `http`, `secrets`, `ai` and `fs`, with examples |
| `references/run-script.md` | How to write a unit for `run_script`: the entry function, its arguments and the result; and running saved scripts and tasks with `run_saved_script` and `run_task` |
| `references/tools.md` | How to write a script tool: `tools/<name>.wcl`, its script and the params object |
| `references/limits-and-errors.md` | The run limits, the error kinds a run reports, and how to fix each |
| `references/bulk-actions.md` | Recipes for changing many notes: query, page, dry run, then apply |

## Working method

1. Read the notes and types you will touch first (`get_note`, `get_type`, `query_notes`), so
   the field names and kinds are right.
2. For a change to many notes, first run a dry run that returns what it would change. Show the
   user, then run the real change. See `references/bulk-actions.md`.
3. Keep the script small. Return a summary `Value`, such as a count and the ids changed.
4. On an error, read its kind and message, fix the code, and run it again.
