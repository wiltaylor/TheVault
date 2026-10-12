---
id: "01"
kind: research
---
# WCL and wscript capabilities

## Question

Can WCL be TheVault's on-disk note format and wscript its handler/scheduled-task language? The note covers WCL's data and schema model, its Rust read, validate and write API, how it handles rich content, wscript's embedding model, and the risks for a multi-user web app.

Sources: WCL repo `/home/wil/dev/wcl` (last commit 2026-10-08, tags to `v0.9.0-alpha`; installed binary `wcl 0.33.2-alpha-211`). The WCL skill is at `~/.claude/skills/wcl` (abbreviated `skill/` below). wscript was **not** found under `/home/wil/dev` as its own repo. It is a dependency of `/home/wil/dev/vmlab` and `/home/wil/dev/config-weave` (GitHub `Configweave/wscript`), and a full checkout lives at **`/home/wil/orca/wscript`** (abbreviated `ws/`). Behaviour marked "probe" was run against the `wcl` binary in a scratch dir.

## Answer

### 1. What WCL is

- WCL is a typed data and configuration language. A document declares its schema alongside its data, and `wcl check` validates the data against it (`wcl/README.md:3-5`). The design goals are "any shape of data" and "strict schemas" (`wcl/CLAUDE.md:4-8`).
- Built-in types are `bool`, `i8`..`i128`, `u8`..`u128`, `f32`/`f64`, `utf8`/`ascii`/`utf16`/`utf32`, `symbol` and `identifier` (`skill/references/language/lang_types.md:9-16`). WCL also has records, aliases, unions, interfaces, optionals `T?`, symbol sets (enums) and function types (`lang_types.md:44,61,94,163,251,296,323`). There are **no date/time, URL or binary/bytes types**; a grep of the types and builtins references found none.
- Schemas are ordinary `type` declarations with structural decorators: `@block(kind)`, `@inline(n)`, `@child`/`@children(kind,min,max)`, `@default`, `@table`, `@document` and `@schemaless` (`lang_decorators.md:47-58`, `lang_schemas.md:11-35`). Constraints are `@min`, `@max` and `@non_empty` only (`lang_types.md:82-93`).
- **A user-defined note type can be expressed and validated.** Probe: a `@block("note") type Note {title: utf8, tags: list<symbol>, @ref("note") links: list<identifier>, body: utf8}` schema plus `note a {...}` instances passed `wcl check`. A dangling link failed with `@ref("note") target 'zzz' is not the id of any 'note' block`, exit 2.
- Custom decorators: `@decorator("name")` makes a type the argument schema of a new decorator, and `@applies_to` restricts where it can appear (`lang_decorators.md:140-170`). Rust hosts can also declare decorators (`lang_decorators.md:345`; `DecoratorBuilder` is exported at `crates/wcl_lang/src/lib.rs:88-90`).
- References come in three forms (`lang_connections.md:229-238`):
  - `@ref(kind)` checks an id against existing blocks (`lang_connections.md:195-227`).
  - `&T` is a lazy typed reference to an interface (`lang_types.md:188`).
  - `connection` declarations with `a -> b :tag` statements project into edge records (`lang_connections.md:1-12`).
- Probe: `@ref` resolves only across files the document **imports**. With one note per file, `c.wcl` linking to `d` failed until `c.wcl` added `import "./d.wcl"`.
- Fields can hold expressions and functions, not only literals (`skill/SKILL.md:15-16`). Imports are either disk-relative paths or host-registered `<system>` imports (`lang_namespaces.md:131-154`).

### 2. Rust crate API

- The workspace has four crates (`wcl/CLAUDE.md:30-41`):
  - `wcl_lang` is the library: parser, evaluator, schema validation, formatter and edit API.
  - `wcl` is the CLI.
  - `wcl_lsp` is the language server.
  - `wcl_wdoc` is the document renderer.
- `wcl_lang` has two deliberately disjoint paths (`crates/wcl_lang/src/lib.rs:1-21`):
  - **Evaluating path:** `Document::open`/`open_with`/`open_at_with_loader` returns a lazy, immutable view (`src/doc/open.rs:28-56`). Validation is `doc.schema_diagnostics()` and `doc.schema_warnings()` (`src/doc/strict.rs:69,186`). Values come out as `wcl_lang::Value` (`src/value.rs:15`).
  - **Edit path:** `parse_for_edit` returns an owned AST with public fields. You mutate it, then `format::to_source` prints it (`src/edit.rs:63`, `src/format/mod.rs:89-95`). `edit::set_field`/`locate_field`/`replace_field` wrap one field edit (`src/edit.rs:256-325`). `parse_for_edit_recovering` tolerates syntax errors (`src/edit.rs:84`).
- **Round-trip is NOT format-preserving.** The printer keeps only line comments and blank lines. Indentation, brace style, number radix and string delimiters are canonicalised (`src/format/mod.rs:7-11`). The `//` versus `#` comment prefix is not kept (`src/ast/trivia.rs:20-24`). Runs of blank lines collapse to one (`trivia.rs:25-27`). `replace_field` re-prints the **whole file** through `to_source` rather than splicing the changed span (`src/edit.rs:290-304`).
- Probe: `wcl set n.wcl notes.a.title '"Hi"'` rewrote unrelated lines. It split `@block("note") type Note {` across two lines, expanded a one-line `note b {...}`, turned `//` into `#`, joined a multi-line list, and renamed and re-indented the heredoc. The comments themselves survived.
- `wcl set` does **not** schema-check. Probe: it wrote successfully into a file that already failed `wcl check`.
- Formatter: `wcl fmt [--in-place] [--indent N]` writes atomically (`lang_cli.md:216-225`). Because `fmt` and `set` produce the same canonical form, a file that is always kept formatted round-trips stably.
- `Value` is Serialize-only. Deliberately there is **no Deserialize**, so JSON cannot be turned back into a `Value` (`src/value.rs:5-8`, `lang_evaluation.md:169-174`). A UI edit has to travel as a WCL expression string or as AST nodes.
- `diff::diff_documents` compares documents semantically, so formatting-only changes produce no diff (`src/lib.rs:19-21`, `lang_cli.md:300`). It could detect real conflicts.
- Stability: the README says "pre-release … APIs may change" (`README.md:11`). Growing enums are `#[non_exhaustive]` (`src/lib.rs:23-39`). The workspace version is `0.0.0` and `wcl_lang` is a git dependency only (`Cargo.toml:6,16`; vmlab pins it by rev, `vmlab/Cargo.toml` `[dependencies]`).

### 3. Large or rich content and assets

- Heredocs: `<<TAG` with escapes, `$<<TAG` with interpolation, and `<<'TAG'` raw/verbatim (`lang_values.md:276-303`). Probe: a Markdown body in a heredoc evaluated back exactly, including inner indentation.
- `len` counts characters, not bytes (`lang_builtins.md:298`). There is no binary or bytes type. Asset references would have to be `utf8` paths or ids validated by convention. wdoc's `image`/`file` blocks reference files by path (`skill/SKILL.md` router: `wdoc_media.md`).
- The formatter may **re-wrap long string labels into heredocs** for prose kinds, a value-changing edit. Today this applies only to kind `p` (`src/format/mod.rs:40-46,59`).
- Hard limits: eval depth 200, parse depth 128, expression depth 256, expanded blocks 100,000 (`src/doc/eval_stack.rs:38`, `src/parser/mod.rs:119,125`, `src/doc/views/mod.rs:87`).

### 4. wscript

- An embeddable, statically typed, Rust-flavoured language compiled to bytecode on a register VM, with the role Lua has for C (`ws/README.md:3-5`). Host-API misuse is a compile-time error (`ws/README.md:32-35`).
- Crates (`ws/README.md:56-66`):
  - `wscript` is the embedding API.
  - `-core`, `-compiler` and `-vm` are the pipeline.
  - `-std` is the standard library.
  - `-macros` provides `#[derive(Script)]`.
  - `-cli` provides `run`, `check`, `repl` and `lsp`.
- Maturity: version 0.2.0 (`ws/Cargo.toml:16-21`), 72 commits, last commit 2026-08-01. "v1 complete: all seven PRD milestones" (`ws/CLAUDE.md:123-127`). The bytecode is "internal and unstable" (`ws/docs/PRD.md:222`). vmlab vendors patched copies of two crates (`vmlab/vendor/README.md`).
- Embedding:
  - `Context` (registrations) is `Send+Sync` and cheap to clone (`ws/wscript/src/lib.rs:106-112`).
  - `Vm` is `!Send`, one per thread (`lib.rs:261-263`; PRD `:25,202-203`).
  - Host functions are registered with `Module::fn_(name, closure)` (`concept_embedding.md:16-17`). Host functions can accept script closures, with re-entry capped at 32 (`concept_embedding.md:123`).
  - `Session` supports multi-file projects (`ws/wscript/src/session.rs:44-93`).
- Sandboxing:
  - Capability-style: "nothing ambient". Every stdlib module is opt-in and feature-gated (`ws/docs/PRD.md:294`, `concept_embedding.md:62`, `ws/wscript-std/Cargo.toml:8-18`).
  - Fuel metering: `Vm::set_fuel(Some(n))`, one unit per instruction. Exhaustion is a trappable fault. Work done inside a host call is **not metered** (`ws/wscript/src/lib.rs:312-330`).
  - Call-depth limit: `set_call_depth_limit` (`lib.rs:295-301`).
  - **No memory limit** was found; a grep of `wscript-vm` for memory or heap limits found none. Reference cycles leak until the VM is dropped (`ws/README.md:52-54`).
- **No async.** Async is an explicit v1 non-goal (`ws/docs/PRD.md:24`). `time::sleep` blocks the thread (`ws/wscript-std/src/time_mod.rs:41-42`).
- **No HTTP module.** The standard library has math, string, fs, process, json/toml/xml, regex and time (`ws/docs/PRD.md:296-305`, `ws/wscript-std/src/`). A grep for http, reqwest and ureq found nothing. TheVault would have to register its own blocking `http` host module.
- Relation to WCL: they are sibling projects by the same author. wscript's docs are authored in WCL (`.claude/skills/wscript/references/entity_wcl.md`). The languages share no runtime. In config-weave, "WCL and wscript never interact directly" and the Rust host mediates (`config-weave/README.md:11-16`). The json/toml/xml `Value` is wscript's only dynamic type (PRD `:307`), and there is no WCL bridge. TheVault would have to convert notes to typed host structs or to that `Value` itself.

## Consequences

- **Clobbering risk (critical).** Any programmatic write re-canonicalises the whole file. If agents hand-write non-canonical WCL and the UI then saves, users see large diffs and lose some layout. Mitigations:
  - Run `wcl fmt` on every write, from both the CLI and the UI, so every stored file is canonical.
  - Or build a span-splicing editor on top of the AST spans, which is upstream work in `wcl_lang`.
  - Decide which one before committing to "agents edit via CLI".
- **Concurrent edits.** WCL has no merge or locking. Use `diff_documents` on the evaluated view plus optimistic version checks per file. The PRD needs a conflict policy.
- **Validation must be explicit.** `wcl set` does not validate, so the backend must run `schema_diagnostics()` before committing any write from an agent or the UI.
- **Cross-note links need imports.** `@ref` only sees imported files. Options:
  - One file per type or vault, which makes large files and heavy contention.
  - Generated import manifests per note.
  - Validating links in TheVault's own index rather than in WCL.
- **Missing types.** There are no date/time, URL or bytes types, and only min/max/non_empty constraints, so there are no regex or length limits on strings. Timestamps, attachments and rich validation need conventions: `utf8` ISO strings, custom decorators the host enforces, or upstream additions.
- **Untrusted documents are executable.** Fields can be expressions or functions and can import files. The host must supply a restricted `FileLoader`, because `overlay_loader` falls through to the real disk (`src/doc/loader.rs:42-58`). It also needs depth and size caps per document.
- **Rich text.** Markdown-in-heredoc works. A WYSIWYG editor needs a chosen body format (Markdown, or wdoc blocks) and must not use kind `p` or any other prose-wrap kind for verbatim fields.
- **wscript in a multi-user server:**
  - Use one `Vm` per worker thread (`!Send`) on a blocking pool, with per-run fuel.
  - There is no memory cap and no async, and fuel does not cover time spent in host calls. HTTP and other I/O need wall-clock timeouts, size limits and SSRF/egress controls on the host side.
  - Never register the `fs` or `process` modules for user scripts.
- **Ownership and stability.** Both projects are pre-1.0, single-author and git-pinned. Breaking changes are likely, and vmlab already carries a local wscript patch. Open questions:
  - Whether the user owns both upstreams and can add span-preserving edits, date types and an HTTP module there.
  - Whether wscript's canonical location is `/home/wil/orca/wscript` or GitHub `Configweave/wscript`.
