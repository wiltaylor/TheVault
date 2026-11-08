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

- `run_script`: run your own wscript now, with full access to the vault and, when you may read
  web pages, the web. It reads no secret: a script tool or a task names the secrets it may read.
  Up to five minutes. Its changes are made at once, as you, with no proposal. From
  **Run wscript code**, which only an admin turns on.
- `list_scripts`, `run_saved_script`, `list_tasks` and `run_task`: run the vault's saved scripts
  by function, and start a scheduled task now. From **Run saved scripts and tasks** (or Run wscript
  code).
- `list_tools`, `get_tool`, `propose_tool_change` and `propose_tool_delete`: read and propose
  script tools. A proposal waits for the user unless **Ask before changing script tools** is off.
- `get_script`, `propose_script_change` and `propose_script_delete`: read a saved script with
  what runs it, and propose a new or changed script, or a delete. The proposal always waits for
  the user's Apply, whatever your switches say, so a trigger's run is not offered them. A change
  must compile in every way the script runs; a delete is refused while a handler, task, device
  file or tool names the script. Changing a shared script changes every one of them, with their
  hosts and secrets: the user sees that list on the card.
- `get_task`, `propose_task_change` and `propose_task_delete`: read a scheduled task with its next
  runs and script, and propose a new or changed task file, with its script's whole source when
  that changes too, or a delete. The proposal always waits for the user's Apply, whatever your
  switches say, so a trigger's run is not offered them. Never give `read_paths` or `write_paths`:
  a task with them is refused, and only the user adds folders, in the task editor. Once applied,
  a task that is not paused runs on its schedule. A delete keeps the script and the run history.
- `start_subagent`: hand a self-contained part of the task to another agent, which works in its
  own thread and answers you. From **Sub agents**.
- `notify_user`: send the user a notification now: a title, an optional body and an optional
  note it opens. It shows under the bell as yours and is pushed to the paired phones, with no
  proposal, also in an unattended run. At most 3 per turn, 10 per hour and 30 per day. Every
  agent has it unless its tools list leaves it out.
- `ask_user`: ask the user a question you cannot go on without, and end the run. Only in a call
  from a note or notes sent to you, where no one watches; in a chat, ask in your answer. The
  thread then shows that it needs the user and the bell tells them, and their answer comes as
  the next message. Every agent has it there unless its tools list leaves it out.

There is no tool to write a device file. For one, write the file's text in your answer for the
user to save, or say what they need to set up.

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
- The function takes no arguments.
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

- Something should happen on a schedule: a scheduled task. Propose it with
  `propose_task_change`, its script in `script_source`, or write both for the user when you lack
  the tool.
- Something should happen whenever a note of a type changes: a handler, through
  `propose_type_change`. To have an agent act instead, the user adds a trigger to that agent.
- The agents keep needing the same lookup or web call: a script tool.
- A one-off change to many notes: `run_script` when you have it, after saying what it will change.
  Without it, propose the changes one note at a time with `propose_fields`, or tell the user.
- A reminder at a time: a task with a due date or a datetime on a note the user sees, or a
  scheduled task that notifies. You cannot set a reminder for later yourself.
- Telling the user something now, such as what an unattended run found: `notify_user`. Something
  to tell them on a schedule, every time: a scheduled task with `vault::notify`.
