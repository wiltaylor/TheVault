# The vault's host modules

A script reaches the vault and the outside world through host modules. `use` each one the script
calls. `run_script` and script tools have `vault`, `http`, `ai`, `secrets` and `fs`, besides the
standard `value`, `json`, `math` and `regex` (see `language.md`).

| Module | `run_script` | Script tools | `before_*` handlers | `after_*` handlers | Tasks |
|--------|--------------|--------------|---------------------|--------------------|-------|
| `vault` | yes | yes | yes | yes | yes |
| `http` | any host | the tool's `allow_domains` | no | the handler's `allow_domains` | the task's `allow_domains` |
| `ai` | yes | yes | no | yes | yes |
| `secrets` | every secret | the tool's `secrets` | no | the handler's `secrets` | the task's `secrets` |
| `fs` | no paths | no paths | no | no | the task's `read_paths` and `write_paths` |

A module a run lacks does not compile.

## Conventions

- Every function that can fail returns `Result[T, string]`, except `ai`, whose error is an
  `AiError`. Use `?` or `match`.
- A note is a `Value`: `{ id, type, version, note }`, where `note` is `{ fields, blocks }`.
- A query row is `{ id, type, title, fields }`.
- A `version` argument guards a write: it fails when the note changed since that version. Where
  a function allows it, `""` takes the note as it is now.
- Writes go through the normal write path: they are validated, kept in note history and run the
  type's handlers. `run_script` and script tools write as the agent that ran them.

## `vault`: reading

| Function | Returns |
|----------|---------|
| `vault::get(id: string)` | `Result[Value, string]`: the note `{ id, type, version, note }` |
| `vault::query(text: string)` | `Result[List[Value], string]`: every row. More than 10,000 rows is an error; add `limit` or page |
| `vault::query_page(text: string, size: int, cursor: string)` | `Result[Value, string]`: `{ rows, total, next }`. `""` is the first page; pass `next` for the one after; `next` is null on the last page. `size` is 1 to 1000 |
| `vault::search(text: string, limit: int)` | `Result[List[Value], string]`: full-text search over notes that are not archived |
| `vault::search_archived(text: string, limit: int, archived: string)` | `Result[List[Value], string]`: archived notes left out (`"exclude"`), kept (`"include"`) or alone (`"only"`) |
| `vault::links(id: string)` | `Result[List[Value], string]`: the notes `id` links to, `{ id, type, title, via, field }` |
| `vault::backlinks(id: string)` | `Result[List[Value], string]`: the notes that link to `id` |
| `vault::types()` | `Result[List[Value], string]`: every type, `{ name, label, plural, description, fields }` |
| `vault::taxonomies()` | `Result[List[Value], string]`: every taxonomy, `{ name, label, description, allow_new, terms }` |

The query language: `from <type>[, <type>] [include archived] [where <condition>] [sort <field>
asc|desc] [limit <n>]`. Conditions use `== != < <= > >=`, `&&`, `||`, `!field` for a false or
empty field, and `today()`, as in `from task where !done && due < today() sort due asc`. A query
leaves archived notes out unless it says `include archived` or names `archived` in its `where`.

```wscript run
use vault

fn main() -> Result[Value, string] {
    let page = vault::query_page("from task where !done sort title asc", 100, "")?
    let total = page.get("total").unwrap().as_int().unwrap()
    let titles = []
    for row in page.get("rows").unwrap().as_list().unwrap() {
        titles.push(row.get("title").unwrap())
    }
    let hits = vault::search("report", 5)?
    Ok(Value::Map(#{
        "open": Value::Int(total),
        "titles": Value::List(titles),
        "hits": Value::Int(hits.len()),
    }))
}
```

Read one note's fields through `note.fields`:

```wscript run
use vault

fn main() -> Result[string, string] {
    let rows = vault::query("from task limit 1")?
    let id = rows[0].get("id").unwrap().as_string().unwrap()
    let note = vault::get(id)?
    let fields = note.get("note").unwrap().get("fields").unwrap()
    // A field the note does not hold is absent: match on the Option.
    let status = match fields.get("status") {
        Some(Value::String(s)) => s,
        _ => "(no status)",
    }
    let done = fields.get("done").unwrap_or(Value::Bool(false)).as_bool().unwrap_or(false)
    Ok(status + " " + str(done))
}
```

## `vault`: changing notes

| Function | Returns |
|----------|---------|
| `vault::create(type_name: string, note: Value)` | `Result[Value, string]`: the new note; `note` is `{ fields, blocks }` |
| `vault::update(id: string, note: Value, version: string)` | `Result[Value, string]`: replace the note whole; `version` is required |
| `vault::patch(id: string, fields: Value, version: string)` | `Result[Value, string]`: set some fields, keep the rest and the blocks. `Value::Null` removes a field |
| `vault::append_text(id: string, field: string, text: string, version: string)` | `Result[Value, string]`: append paragraphs of inline markup to a content field |
| `vault::append_blocks(id: string, field: string, blocks: Value, version: string)` | `Result[Value, string]`: append one content block or a list, as `{ kind, fields, blocks }` |
| `vault::delete(id: string, version: string)` | `Result[Value, string]`: move the note to the trash. `version` is required |
| `vault::restore(id: string)` | `Result[Value, string]`: take a note out of the trash. Nothing empties the trash |
| `vault::archive(id: string)` / `vault::unarchive(id: string)` | `Result[Value, string]` |
| `vault::daily(date: string)` | `Result[Value, string]`: the daily note for `YYYY-MM-DD`, made when missing; `""` is today |
| `vault::add_term(taxonomy: string, path: string)` | `Result[Value, string]`: add a term to an open taxonomy |

```wscript run
use vault

fn main() -> Result[Value, string] {
    let made = vault::create("task", Value::Map(#{
        "fields": Value::Map(#{
            "title": Value::String("Weekly review"),
            "priority": Value::Int(1),
        }),
    }))?;
    let id = made.get("id").unwrap().as_string().unwrap()
    let patched = vault::patch(id, Value::Map(#{ "status": Value::String("Todo") }), "")?
    let version = patched.get("version").unwrap().as_string().unwrap()
    let appended = vault::append_text(id, "body", "First step: list the open tasks.", version)?
    // delete needs the note's current version: "" is not accepted here.
    vault::delete(id, appended.get("version").unwrap().as_string().unwrap())
}
```

A block's form is `{ kind, fields, blocks }`, such as a paragraph
`{ kind: "paragraph", fields: { text } }` or an admonition
`{ kind: "admonition", fields: { kind: "warning", title }, blocks: [...] }`:

```wscript
use vault

fn add_warning(id: string, text: string) -> Result[Value, string] {
    let callout = Value::Map(#{
        "kind": Value::String("admonition"),
        "fields": Value::Map(#{ "kind": Value::String("warning"), "title": Value::String("Check") }),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("paragraph"),
            "fields": Value::Map(#{ "text": Value::String(text) }),
        })]),
    })
    vault::append_blocks(id, "body", callout, "")
}
```

## `vault`: assets, notifications, time and the log

| Function | Returns |
|----------|---------|
| `vault::assets()` | `Result[List[Value], string]`: `{ id, filename, size }` each |
| `vault::read_asset(id: string)` | `Result[Value, string]`: `{ id, filename, size, base64 }` |
| `vault::upload_asset(filename: string, base64: string)` | `Result[Value, string]`: a new asset |
| `vault::notify(title: string, body: string, note: string)` | `Result[Value, string]`: a notification under the bell; `""` for no body or note |
| `vault::device_command(device: string, name: string, payload: Value)` | `Result[Value, string]`: queue a command for a paired device |
| `vault::now()` | `int`: milliseconds since the Unix epoch |
| `vault::today()` | `string`: today as `YYYY-MM-DD` |
| `vault::log(message: string)` | Adds a line to the run's log; `print` and `println` write there too |

```wscript run
use vault

fn main() -> Result[string, string] {
    let started = vault::now()
    vault::log("today is " + vault::today())
    println("took {vault::now() - started} ms")
    Ok(vault::today())
}
```

## `http`

| Function | Returns |
|----------|---------|
| `http::get(url: string, headers: Map[string, string])` | `Result[Response, string]` |
| `http::post(url: string, headers: Map[string, string], body: string)` | `Result[Response, string]` |
| `http::request(method: string, url: string, headers: Map[string, string], body: string)` | `Result[Response, string]` |
| `http::base64(text: string)` / `http::base64url(text: string)` | `string` |
| `http::wrap(text: string, width: int)` | `string`: lines of `width` joined by CRLF |

`Response` is `{ status: int, headers: Map[string, string], body: string, body_base64: string }`
with lower-case header names. A 404 is a response, not an error: check `status`. Redirects are
never followed. A response body over 1 MiB is an error.

```wscript
use http
use json

fn weather(city: string) -> Result[string, string] {
    let r = http::get("https://api.example.com/weather?city=" + city, #{ "accept": "application/json" })?
    if r.status != 200 {
        return Err(fmt("the weather API answered {}", r.status))
    }
    let data = json::parse(r.body)?
    Ok(data.get("sky").unwrap().as_string().unwrap_or("unknown"))
}

fn post_json(url: string, payload: Value) -> Result[int, string] {
    let r = http::post(url, #{ "content-type": "application/json" }, json::to_string(payload))?
    Ok(r.status)
}
```

## `secrets`

| Function | Returns |
|----------|---------|
| `secrets::get(name: string)` | `Result[string, string]`: a stored secret's value |
| `secrets::set(name: string, value: string)` | `Result[(), string]`: store a value, at most 64 KiB |

A tool or task reads only the secrets it names; `run_script` may read any. No function lists
them. Never return or log a secret's value.

```wscript
use http
use secrets

fn call_api(path: string) -> Result[string, string] {
    let key = secrets::get("example_api_key")?
    let r = http::get("https://api.example.com" + path, #{ "authorization": "Bearer " + key })?
    Ok(r.body)
}
```

## `ai`

| Function | Returns |
|----------|---------|
| `ai::complete(prompt: string, options: Map[string, string])` | `Result[Reply, AiError]` |
| `ai::chat(messages: List[ChatMessage], options: Map[string, string])` | `Result[Reply, AiError]` |

`ChatMessage` is `ChatMessage { role: "user", text: "..." }` (`user` or `assistant`). `Reply` is
`{ text, json, account, kind, model, input_tokens, output_tokens, stop }`. `AiError` is
`{ kind, message, account, model, category }`. Options, all strings: `account`, `model`,
`system`, `schema` (a JSON Schema as JSON text; the answer arrives parsed in `json`), `effort`,
`max_tokens`. An unknown option is an error.

```wscript
use ai
use json

fn summarise(text: string) -> Result[string, string] {
    let schema = json::to_string(Value::Map(#{
        "type": Value::String("object"),
        "properties": Value::Map(#{ "summary": Value::Map(#{ "type": Value::String("string") }) }),
        "required": Value::List([Value::String("summary")]),
        "additionalProperties": Value::Bool(false),
    }))
    let reply = match ai::complete("Summarise in one sentence:\n" + text, #{ "schema": schema, "max_tokens": "300" }) {
        Ok(r) => r,
        Err(e) => return Err(e.kind + ": " + e.message),
    }
    Ok(reply.json.get("summary").unwrap().as_string().unwrap())
}

fn ask(question: string) -> Result[string, string] {
    let messages = [ChatMessage { role: "user", text: question }]
    match ai::chat(messages, #{}) {
        Ok(r) => Ok(r.text),
        Err(e) => Err(e.message),
    }
}
```

## `fs`

Only a scheduled task has file access, inside its `read_paths` and `write_paths`; `run_script`
and tools have the module but no paths, so every call is refused. Paths must be absolute, and
nothing inside the vault folder is reachable: change notes with `vault`.

| Function | Returns |
|----------|---------|
| `fs::read_text(path)` / `fs::read_bytes(path)` | `Result[string, string]` (bytes in base64) |
| `fs::write_text(path, text)` / `fs::append_text(path, text)` / `fs::write_bytes(path, base64)` | `Result[(), string]` |
| `fs::list(path)` | `Result[List[Entry], string]`, `Entry` `{ name, path, is_dir, size, modified }` |
| `fs::exists(path)` | `Result[bool, string]` |
| `fs::mkdir(path)` / `fs::remove(path)` | `Result[(), string]` |
| `fs::join(base, name)` | `string` |

```wscript
use fs

fn export(dir: string, rows: List[Value]) -> Result[int, string] {
    let out = fs::join(dir, "export.csv")
    fs::write_text(out, "id,title\n")?;
    for row in rows {
        let id = row.get("id").unwrap().as_string().unwrap()
        let title = row.get("title").unwrap().as_string().unwrap_or("")
        fs::append_text(out, id + "," + title + "\n")?;
    }
    Ok(rows.len())
}
```
