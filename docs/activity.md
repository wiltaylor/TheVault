# The activity log

The activity log records what happens in the vault: note edits, changes to types and other definitions, AI turns and every tool call an agent makes, scheduled task runs, the Outbox, sync and sign-ins. Open it from **Log** in the side menu, under **Outbox**, to see what was done and by whom. It is most useful for watching what an AI agent does.

## The Log page

The page lists entries newest first, grouped by day under **Today**, **Yesterday** and then the date. Each entry shows:

- the time;
- who did it: **You**, an agent with its icon, a script, a task, a device, a token, or the server;
- an icon for its category;
- a one-line summary, such as the note's type and title or the tool an agent called;
- a link to what it is about: the note, the agent's thread, the scheduled task, the Outbox item, the type, the taxonomy or the agent.

Select an entry to expand it. The expanded entry shows its kind (such as `note.update` or `ai.tool`), the full date and time, the actor and the subject, and its detail fields: for example the tool name and a short summary of its arguments, the tokens a turn used and how it ended, or a note's new version.

An AI turn shows the tool calls it made nested under it, with its proposals, memory changes and the end of the turn, when the turn's start is on the page.

Scroll down to load older entries. New entries appear at the top as they happen.

## Filters

The toolbar above the list filters it. The filters are kept in the page's address, so a filtered Log can be bookmarked or opened in another pane.

- **Category chips**: **All**, **AI**, **Notes**, **Definitions**, **Automation**, **Outbox**, **Sync** and **Auth**. **AI** shows the AI activity: chat turns, trigger and call runs, tool calls, proposals and what became of them, agent memory and browser commands.
- **Actor**: **Everyone**, **You**, each agent, each script and scheduled task, each device, and anyone else whose entries are on the page.
- **Search the log**: shows the entries whose summary contains the text. It searches as you pause typing, or at once on <kbd>Enter</kbd>.
- **Pause**: stops new entries from moving the list while you read it. New entries wait behind a **N new entries: show** button. **Resume**, or that button, adds them.

## Watching an agent

**Show in log** on an agent's screen, and in the **⋯** menu of each of its threads, opens the Log on that agent's AI activity: the **AI** category with the agent as the actor. The address is `/log?category=ai&actor=agent:<name>`.

## In the desktop and Android apps

The apps read the newest entries again every 10 seconds while the Log is open, instead of being told about each new entry. The log needs the `admin` scope on the home server. Where the app's link to your server cannot read it, the page says to open the log in the web app.

## What is kept

The log keeps entries for 90 days, and at most the newest 200 000 entries. Older entries are removed when the server starts and once a day.

## Privacy

The log records that something happened, not what was said or stored:

- It never copies the text of chat messages or of a model's answers. An AI entry links to its thread, where the conversation is.
- It never holds secret values, passwords or tokens.
- An Outbox entry names the recipients, not the message body.
- A tool call is summarised: the search text, the note titles, the host of a web address, the first 120 characters of a command.

The log lives in the server's `state.db`, not in the vault's note files, so devices do not sync it.

## What is recorded

Each entry has a dotted kind, a category, an actor, a one-line summary, an optional subject it links to, and a small detail object.

| Category | Kinds | Recorded when |
|---|---|---|
| `notes` | `note.create`, `note.update`, `note.archive`, `note.unarchive`, `note.delete`, `note.restore`, `note.purge` | Any note write, whoever makes it: the web app, the chat and agents, scripts, tasks and device sync. One actor's edits of one note within 60 seconds are one `note.update` entry, its `detail.edits` counting them. The detail holds the note's `type`, `title` and new `version`. |
| `definitions` | `type.*`, `taxonomy.*`, `agent.*`, `task.*`, `script.*`, each `create`, `update` or `delete` | A type, taxonomy, agent, task or script file is written or deleted. A template change is a change of its type. |
| `ai` | `ai.turn.start`, `ai.turn.end` | Each chat turn, and each trigger or call run's turn. The detail holds the `agent`, the `thread`, the `origin` (`user`, `trigger:<name>` or `mention`) and the `turn` id; the end adds the `provider`, `model`, `input_tokens`, `output_tokens`, `ms`, the number of `tools` called and the `outcome` (`done`, `refused`, `error` or `stopped`) with `why`. |
| `ai` | `ai.tool`, `ai.browser` | Each tool call of a turn: `tool`, `args` (only search text, ids, names, titles, a web address without its query, the first 120 characters of a command), the `notes` it named and `is_error`. A browser command adds its `exit`, `ms` and `timed_out`. |
| `ai` | `ai.proposal`, `ai.applied`, `ai.discarded` | A turn proposed a change, or made one itself with its confirmation off; the user applied (`detail.ok`) or discarded a proposal. |
| `ai` | `ai.memory` | An agent memory was remembered, updated or forgotten, by the agent's own tools, another agent's proposal or the Memory tab: the memory's title, never its text. |
| `automation` | `task.run.start`, `task.run.end` | A scheduled task run starts and ends, with the run id, the task, the attempt and the outcome. |
| `automation` | `agent.run`, `agent.run.skipped` | An agent trigger or call run ended, or was skipped over the hourly limit. |
| `automation` | `handler.failed` | A handler failed: a `before_*` handler that rejected a write, or an `after_*` job that failed or was not run. |
| `outbox` | `outbox.drafted`, `outbox.ready`, `outbox.sending`, `outbox.sent`, `outbox.failed` | An Outbox item was drafted, marked ready, claimed for sending, sent (the write that records `sent_at`) or failed, with its kind and recipients, not its body. A sent item then leaves the Outbox, and a draft deleted (rejected) is a `note.delete`. |
| `sync` | `task.run.start`, `task.run.end` | A run of the Google Gmail or Calendar sync (the `google_gmail` and `google_calendar` tasks), with its outcome. |
| `sync` | `sync.round` | A device finished a sync round that moved or held something: pushed, pulled, merged and conflict counts and the held notes. The app reports it with `POST /api/devices/{id}/sync-rounds`. |
| `auth` | `auth.login`, `auth.login_failed`, `auth.token_created`, `auth.token_revoked`, `auth.device_paired`, `auth.device_revoked` | Password sign-ins and failed sign-ins, API tokens made or revoked (an agent install code makes one too), devices paired or removed. |

The actor is `<kind>:<name>`: `session:owner` for you, `agent:<name>`, `handler:<script>`, `task:<name>`, `token:<name>`, `device:<device name>` for a paired device's token, or `server:server` for what the server does itself, such as writing a shipped default or a failed sign-in.

## API

`GET /api/activity` needs the `admin` scope: a password session, or an `admin` token. A `read` or `write` token, such as a device's, gets 403.

| Parameter | Meaning |
|---|---|
| `category` | One of `ai`, `notes`, `definitions`, `automation`, `outbox`, `sync`, `auth`. Another value is a 400. |
| `actor` | An actor kind, such as `agent`, or a kind and name, such as `agent:researcher` or `session:owner`. |
| `kind` | A kind, such as `ai.tool`, or a dotted prefix of kinds, such as `ai.turn` or `note`. |
| `q` | Text the summary contains, in any case. |
| `before` | An entry id: only entries older than it. Pass the `next` of the page before. |
| `limit` | 1 to 200 entries; 100 when left out. |

The answer is `{items, next}`, newest first. `next` is the `before` of the next page, or `null` on the last. Each item is:

```json
{
  "id": "01K8...",
  "at_ms": 1761300000000,
  "kind": "note.update",
  "category": "notes",
  "actor_kind": "session",
  "actor": "owner",
  "summary": "Edited task \"Plan\"",
  "subject_kind": "note",
  "subject": "01K8...",
  "detail": { "type": "task", "title": "Plan", "version": "...", "edits": 2 }
}
```

`id` is a ULID, so ids sort by time. `subject_kind` is one of `note`, `type`, `taxonomy`, `agent`, `task`, `script`, `thread`, `task_run`, `outbox`, `device` or `token`, and `subject` the bare id or name; both are `null` for an entry about nothing in particular. `summary` is at most 300 characters and `detail` at most 4 KiB.

The live event stream, `GET /api/events`, sends each new entry as the event `activity` with the data `{"item": {...}}`, the item as the list has it. A `note.update` entry that counts another edit comes again with the same `id`: replace the one shown. Only `admin` callers get these events.

The embedded servers of the desktop and Android apps answer `GET /api/activity` with 403 `needs_web_app` at once: the device token has no `admin`, and the device's own `state.db` holds only its local writes.
