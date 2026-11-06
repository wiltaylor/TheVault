---
name: vault-automation
description: Know what TheVault can automate and which of your tools can do it. Use when the user asks for something to happen on a schedule or when a note changes, for a tool that calls a web service, for a reminder or a notification, or for a bulk change; and before calling run_script, run_saved_script, run_task or start_subagent, or proposing a script tool.
---

# Automating the vault

Four things run code in the vault, all written in wscript (open the `wscript` skill for the
language and the vault API):

| What | File | Runs | Who writes it |
|------|------|------|---------------|
| A scheduled task | `tasks/<name>.wcl` and `scripts/<script>.ws` | On a cron schedule, or by Run now | The user, in Settings > Scheduled |
| A handler | A `handler` block in a type file, and a script | When a note of the type is created, changed or deleted | The user, or you with `propose_type_change` |
| A script tool | `tools/<name>.wcl` and a script | When an agent calls it as a tool | You with `propose_tool_change`, or the user in Settings > Tools |
| Your own code | none | Once, now, with `run_script` | You |

Every example here is checked: the WCL parses and the wscript compiles.

## Which tools you have

Each comes from a switch on your Permissions tab, so you may have none of them:

- `run_script`: run your own wscript now, with full access to the vault, the web and every
  secret, for up to five minutes. Its changes are made at once, as you, with no proposal. From
  **Run wscript code**, which only an admin turns on.
- `list_scripts`, `run_saved_script`, `list_tasks` and `run_task`: run the vault's saved scripts
  by function, and start a scheduled task now. From **Run saved scripts and tasks** (or Run wscript
  code).
- `list_tools`, `get_tool`, `propose_tool_change` and `propose_tool_delete`: read and propose
  script tools. A proposal waits for the user unless **Ask before changing script tools** is off.
- `start_subagent`: hand a self-contained part of the task to another agent, which works in its
  own thread and answers you. From **Sub agents**.

There is no tool to write a scheduled task, a saved script or a device file, and none to send
a notification. For those, write the file's text in your answer for the user to save, or say what
they need to set up.

## A scheduled task

The task file names its script and when it runs:

```wcl
label         = "Overdue tasks"
description   = "Every weekday at 7, tell the user how many tasks are overdue."
schedule      = "0 7 * * 1-5"
timezone      = "Australia/Melbourne"
script        = "overdue"
function      = "run"
retries       = 2
```

The script, `scripts/overdue.ws`. A scheduled script tells the user things with `vault::notify`,
which puts a notification under the bell that opens a note:

```wscript
use vault

fn run() -> Result[(), string] {
    let page = vault::query_page("from task where !done && due < today()", 1, "")?
    let overdue = page.get("total").unwrap().as_int().unwrap()
    if overdue == 0 {
        return Ok(())
    }
    let day = vault::daily("")?
    let id = day.get("id").unwrap().as_string().unwrap()
    vault::notify(fmt("{} tasks are overdue", overdue), "See today's note.", id)?
    Ok(())
}
```

- `schedule` is five cron fields: minute, hour, day of month, month, day of week.
- A task reaches the web only through `allow_domains`, and secrets only through `secrets`.
- A failed task with no retry left notifies the user itself.

## A handler

A handler block goes in the type file and names a script and a function:

```wcl
handler "before_create" {
  script   = "invoice"
  function = "before_create"
}

handler "after_create" {
  script        = "invoice"
  function      = "after_create"
  allow_domains = ["hooks.example.com"]
}
```

A `before_*` handler runs inside the write. It answers `Ok(Value::Null)` to keep the note as it
is, a map of `fields` to change it, or an error to refuse the save:

```wscript
fn before_create(note: Value) -> Result[Value, string] {
    let title = note.get("note").unwrap().get("fields").unwrap().get("title").unwrap().as_string().unwrap()
    if title.trim().is_empty() {
        return Err("an invoice needs a title")
    }
    Ok(Value::Null)
}
```

An `after_*` handler runs after the commit, so it may call the web and notify the user:

```wscript
use vault

fn after_create(note: Value) -> Result[(), string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    vault::notify("New invoice", "Check the amount before it is paid.", id)?
    Ok(())
}
```

Handlers are part of the type, so propose them with `propose_type_change` and the whole type file.
A change an agent makes never fires a trigger, but it does run the type's handlers.

## A script tool

A tool file describes the tool to the model and names the function:

```wcl
description = "Count the open tasks that carry a tag. Use it before planning a week."
script      = "count_tagged"
function    = "run"
timeout     = 30

param tag {
  kind        = "string"
  description = "The tag, such as \"Garden\"."
  required    = true
}
```

The function takes one `Value`, a map of every parameter, and answers a `Value`:

```wscript
use vault

fn run(params: Value) -> Result[Value, string] {
    let tag = params.get("tag").unwrap().as_string().unwrap()
    let rows = vault::query(fmt("from task where !done && has(tags, \"{}\")", tag))?
    Ok(Value::Map(#{ "tag": Value::String(tag), "open": Value::Int(rows.len()) }))
}
```

A tool is offered to agents only when it is valid; `get_tool` shows why one is not.

## Choosing

- Something should happen on a schedule: a scheduled task. Write its file and script for the
  user.
- Something should happen whenever a note of a type changes: a handler, through
  `propose_type_change`. To have an agent act instead, the user adds a trigger to that agent.
- The agents keep needing the same lookup or web call: a script tool.
- A one-off change to many notes: `run_script` when you have it, after saying what it will change.
  Without it, propose the changes one note at a time with `propose_fields`, or tell the user.
- A reminder at a time: a task with a due date or a datetime on a note the user sees, or a
  scheduled task that notifies. You cannot set a reminder yourself.
