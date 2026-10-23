# AI agents in the vault

An agent is an AI chat with its own model, system prompt and permissions, defined by a file in the vault. You chat with it in threads, and it can also run on its own: on a schedule, when notes change, or when you call it from a note with `@<name>`. The chat panel's default agent is the built-in **Assistant**.

This page is about agents the vault runs with your own AI provider. To connect an outside AI agent, such as a coding assistant, to the vault through the CLI, see [Connecting an AI agent](agents.md). Setting up the providers themselves is in [AI providers](ai.md).

## Agent files

Each agent is a file `agents/<name>.wcl`. The name is lower case letters, digits and `_`, starting with a letter, at most 64 characters. Agent files are definitions like types and tasks: they keep their history, they are in backups, they sync to the apps, and changing one needs `admin`. The apps show agents read only, with "Edit agents in the web app".

The file names the label, description and icon, the `provider` and `model` (absent means the Settings > AI defaults), the `effort`, the `system_prompt` (absent means the shipped chat prompt), a `permissions` block and any `trigger` blocks. `enabled = false` keeps the file but starts no chat and runs no trigger. [Vault file format, Agent files](vault-format.md#agent-files) defines every item.

In the web app, the left sidebar's **Agents** section lists every agent, Assistant first; **+** creates one. Each agent opens at `/agents/<name>`, with these tabs:

| Tab | Holds |
|---|---|
| Chat | The agent's threads and the open thread |
| Settings | Label, description, icon, enabled, provider, model, effort and system prompt |
| Permissions | Tools, confirmation switches, internet switches and note types |
| Triggers | Schedules, note events and mentions, with each trigger's last runs |
| File | The WCL file itself, checked as the form is |

The Assistant (`agents/assistant.wcl`) is shipped with every vault. It cannot be deleted or disabled; everything else about it can be changed.

## Permissions

An agent's permissions are its own: nothing is inherited from Settings > AI.

- **Tools**: the chat tools the agent is offered. A tool the server cannot offer, such as `web_search` without a Brave Search key, is still left out.
- **Confirmation switches** (`permissions.confirm`): with a switch on, a change of that kind is a proposal you apply or discard; with it off, the agent makes the change itself. See [Changes the AI makes](ai.md#changes-the-ai-makes).
- **Internet switches** (`permissions.web`): which web tools the agent is given at all. See [Internet switches](ai.md#internet-switches).
- **Note types** (`permissions.types`): the types the agent may read and change. Search, query, backlinks and asset results leave out notes of other types, reading one is refused with "This agent may not read <type> notes.", and every change to one is refused.

In a chat, the person or token running it still caps what the agent can change: an agent never writes more than its caller could.

## Threads

Every conversation with an agent is a note of the built-in type `agent_thread`, holding the agent's name, where it came from (`user`, `trigger:<name>` for a trigger run, or `mention` for a call from a note), when it was last active, the transcript and a readable copy of the conversation. The server writes the thread as each turn streams, so closing a tab loses nothing. Proposals and what became of them are kept in the thread, so a reopened thread shows its pending proposals with Apply and Discard. [Threads](ai.md#threads) describes them in full.

The note tools leave threads out unless the agent's `permissions.types` names `agent_thread`, so an agent does not read chats back as vault content. Deleting an agent keeps its threads.

Each thread in the Chat tab has a ⋯ menu, also opened by a right-click, with Rename, Archive or Unarchive, and Delete. These are ordinary note changes, authored by you: Delete moves the thread note to the trash, where it can be restored. Archived threads are hidden from the list until **Show archived**, at its foot, lists them dimmed. An archived thread still opens, and sending a message in it takes it out of the archive. When the chat panel's thread is archived or deleted elsewhere, its next message starts a new thread.

A turn runs on the server until it ends or you press **Stop**, whether or not anything shows it. Opening another thread, choosing another agent in the panel, closing the panel or the tab only stops showing it; the turn goes on writing to its thread. A running thread shows a small spinner beside its title in the Chat tab, and the agent shows one in the sidebar; opening the thread shows the answer so far and follows the rest, with the activity line counting from when the turn started. A thread that answered while it was not open shows a dot, "New reply", until you open it. One thread runs one turn at a time ("Still answering" while it does), and an agent runs at most 4 turns at once across its threads, trigger runs included. [The turns form](ai.md#the-turns-form) describes the routes.

## Triggers

A `trigger` block runs the agent on its own. Each run starts a new thread, titled with the trigger's name and the local date and time, whose first message is the trigger's `prompt`.

### Schedules

`schedule` is a five-field cron expression, read in `timezone` (UTC by default), exactly as a scheduled script's schedule is. A schedule fires once however long the server was down: missed firings are not replayed. The Triggers tab shows the schedule in words and its next run times.

```wcl
trigger nightly {
  schedule = "0 2 * * *"
  timezone = "Australia/Melbourne"
  prompt   = "Summarise the email that came in today."
}
```

### Note events

`on` lists the changes that start a run: `create`, `update` and `delete` (a move to the trash). `type` limits it to one note type or a list of them; without it, every type. `where` is an optional condition in the query language, checked against the note after the change, or before it for a delete.

```wcl
trigger new_email {
  on     = ["create"]
  type   = "email"
  where  = "processed != true"
  prompt = "File this email."
}
```

The run's first message is the prompt followed by a line naming the change:

```text
File this email.
Event: create of note 01JB8Z0Q6W4ZK3V9T1M5XH7N2C (type email, title "Invoice 42")
```

A run starts once the note has been quiet for 30 seconds: several changes to one note within that time make one run, with the last change. A condition that no longer matches by then, or a note gone by then, starts nothing. A trigger with no `type` does not fire for `agent_thread` notes; name the type to watch threads.

Note events fire on the home server only, after the change is written. A change made in the desktop or Android app fires when it reaches the server by sync. The apps themselves never run triggers.

### Calls from notes

A trigger with `mention = true` lets a note call the agent. Write `@` and the agent's name in a note, such as `@clerk list what to buy`, or type `@` in the editor and choose **Ask <label>**, which the picker lists before the notes for each agent the note's type may call. The label calls the agent too when it is one word. `type` limits which note types may call it, and `prompt`, which is optional here, is added before what the note says.

```wcl
trigger asked {
  mention = true
  type    = ["memo", "project"]
  prompt  = "Keep your answer short."
}
```

A call stands in the text of a note's content (paragraphs, headings, list items, quotes, toggles and admonition titles) or in a text field. It needs a space or the start of the text before the `@` and a space, punctuation or the end of the text after the name, so an email address such as `bob@clerk.com` never calls `clerk`. Text in inline code, a link or a note mention never calls anyone. [Vault file format, Agent files](vault-format.md#agent-files) has the exact rule.

Once the note has been idle for 5 seconds (each save starts the wait again), the server answers each agent the note calls, once however many times it is called:

1. It opens a thread, with origin `mention`, titled with the note's title and the start of what was asked.
2. It rewrites the note once, as the agent, replacing each call with a link to the thread: `[@Clerk](/agents/clerk/chat?thread=<id>)`. In a text field, which shows no links, the call becomes `Clerk (asked: /agents/clerk/chat?thread=<id>)`. A call that is a link no longer calls, so saving the note again does not call the agent again.
3. It starts the run. Its first message is the trigger's prompt, then the paragraph or list item that held the call with the call written as "you", then a line naming the note:

```text
Keep your answer short.
you list what to buy for the gutters.
Called from note 01JB8Z0Q6W4ZK3V9T1M5XH7N2C (type memo, title "Roof"); the call was replaced with a link to this thread.
```

The run is unattended, as any trigger run is (below). To let it change the note it was called from, turn off **Ask before creating and editing notes** on the agent's Permissions tab; the Triggers tab warns when the agent could not. Otherwise it answers in the thread, which the link opens.

If the note changes between the server reading it and rewriting it, the call is left where it is, the thread just opened goes to the trash, and the save that changed the note is read for calls in turn. A call written by an agent, including the called agent's own changes to the note, calls no one. Calls are answered on the home server only: a call written in the desktop or Android app is answered when the note reaches the server by sync.

### What an unattended run may do

A trigger run has no one present to apply a proposal, so it is offered only:

- the read tools the agent's permissions allow, and
- the change tools whose confirmation switch is off for the agent.

A change tool whose switch is on is left out of the run's tool list, so nothing is left pending. The Triggers tab lists the tools a run is offered. A run acts as the agent itself: with `write` for notes, and `admin` for types and taxonomies only when its permissions allow those tools. Its permissions are its only limit, as no person or token is running it.

## No loops

Every change an agent makes is authored by the agent (author kind `agent`) in the note's history: changes its tools make, changes you apply from its proposals, its writes to its threads, and the rewrite that replaces a call with a link. A change authored by any agent never fires a note event trigger and is never read for calls, so an agent cannot set itself or another agent off.

Changes by you, by tokens, by scripts, tasks and handlers, and by sync from the apps do fire triggers. A handler that runs because of an agent's change writes as the handler, so it can fire a trigger.

## Limits

| Limit | Value | Past it |
|---|---|---|
| Runs of one agent at once | 2 | Further runs wait their turn |
| Runs waiting for one agent | 50 | The oldest waiting run is dropped, and the server log says so |
| Trigger runs one agent starts in an hour | 60 | Further runs are skipped and logged, with one notification an hour |
| Model calls in one run | 12 | The run stops, as a chat turn does |
| Quiet time before a note event run | 30 seconds | Changes to the same note coalesce |
| Idle time before a note's calls are answered | 5 seconds | Each save starts the wait again |

## Runs and notifications

Every trigger run is recorded in `state.db` (`agent_runs`): the agent, the trigger, the thread, when it started and ended, its outcome and its message, and its [trail lines](ai.md#the-trail-line). The server log has the trail lines and one line per run:

```text
agent run 12: agent=triage trigger=new_email thread=01JB8Z1A2B3C4D5E6F7G8H9J0K outcome=succeeded
```

The Triggers tab shows each trigger's last 20 runs, with a link to each run's thread. A run that fails, through a provider error, a refusal or a turn that does not end, leaves one bell notification, "<Agent>'s <trigger> run failed", that opens its thread. A run that goes well sends none.

## API

| Method and path | Scope | Does |
|---|---|---|
| `GET /api/agents` | read | Every agent's summary, Assistant first, and the tools `permissions.tools` may name. A summary's `mention` is `null`, or `{ types }` for an enabled agent with a mention trigger: the types whose notes may call it, `null` for every type |
| `GET /api/agents/{name}` | read | The parsed agent and its WCL |
| `PUT /api/agents/{name}` | admin | Writes the agent from WCL, `{ wcl }`, or the JSON form `GET` answers |
| `DELETE /api/agents/{name}` | admin | Deletes the agent file; its threads stay |
| `GET /api/agents/{name}/runs` | read | `{ tools, triggers: [{ name, kind, runs }] }`: the tools a trigger run is offered, and each trigger's last 20 runs, newest first, each `{ id, thread, started, ended, outcome, message }` with times in milliseconds |
| `GET /api/agents/{name}/threads` | read | The agent's threads, newest activity first, each with `archived`; archived threads only with `?archived=true` |
| `POST /api/agents/{name}/threads` | read | Starts a thread from `{ title? }` |
| `GET /api/agents/{name}/threads/{id}` | read | A thread with its messages, steps and proposals |
| `PATCH /api/agents/{name}/threads/{id}` | write | Renames a thread from `{ title }` |
| `DELETE /api/agents/{name}/threads/{id}` | write | Moves a thread to the trash |

The chat routes take `agent` and `thread`; see [The chat route](ai.md#the-chat-route). In the apps, the runs and threads routes are answered by the home server.
