# AI agents in the vault

An agent is an AI chat with its own model, system prompt and permissions, defined by a file in the vault. You chat with it in threads, and it can also run on its own: on a schedule, when notes change, or when you call it from a note with the editor's `@` picker. The chat panel's default agent is the built-in **Assistant**.

This page is about agents the vault runs with your own AI accounts. To connect an outside AI agent, such as a coding assistant, to the vault through the CLI, see [Connecting an AI agent](agents.md). Setting up the accounts themselves is in [AI accounts](ai.md).

## Agent files

Each agent is a file `agents/<name>.wcl`. The name is lower case letters, digits and `_`, starting with a letter, at most 64 characters. Agent files are definitions like types and tasks: they keep their history, they are in backups, they sync to the apps, and changing one needs `admin`. The apps show agents read only, with "Edit agents in the web app".

The file names the label, description and icon, the `account` and `model` (absent means the default account in Settings > AI and its model; the chat box's account and model menus can override them for a conversation, but runs the chat box does not start always use them), the `effort`, the `system_prompt` (absent means the shipped chat prompt), a `permissions` block and any `trigger` blocks. `enabled = false` keeps the file but starts no chat and runs no trigger.

`account` names an AI account by its name in Settings > AI (accounts A6). The accounts live in the server's secrets, not in the vault, so nothing checks the name when the file is saved: an agent naming an account that does not exist runs on the default account, and its General tab says "Account <name> does not exist; this agent uses the default account." Renaming an account in Settings > AI rewrites every agent file naming it. Deleting an account the agents name asks first, listing them; they then run on the default account. A file from before accounts may say `provider = "anthropic"`, `"openai"` or `"chatgpt"` instead: it runs on the account made from that provider, and the home server rewrites it to `account` on its next start. [Vault file format, Agent files](vault-format.md#agent-files) defines every item.

In the web app, the left sidebar's **Agents** section lists every agent, Assistant first; **+** creates one. Each agent opens at `/agents/<name>` on its chat: a bar with the session picker, Rename, Archive and Delete for the chosen session and the settings cog, then the chosen session filling the pane, with the account and model menus at the foot of the message box. [Threads](ai.md#threads) describes the chat. The cog opens the agent's settings at `/agents/<name>/settings`, with these tabs, each at `/agents/<name>/settings/<tab>`; the arrow at the top goes back to the chat:

| Tab | Holds |
|---|---|
| General | Label, description, icon, enabled, account, model, effort and system prompt |
| Permissions | Tools, confirmation switches, internet switches, skills and note types |
| Memory | What the agent remembers |
| Triggers | Schedules, note events and mentions, with each trigger's last runs |
| File | The WCL file itself, checked as the form is |

The addresses of the screen's earlier tabs still work: `/agents/<name>/chat` opens the chat, and `/agents/<name>/memory`, `/permissions`, `/triggers` and `/file` open those settings tabs.

The Assistant (`agents/assistant.wcl`) is shipped with every vault. It cannot be deleted or disabled; everything else about it can be changed.

## Permissions

An agent's permissions are its own: nothing is inherited from Settings > AI.

- **Tools**: the chat tools the agent is offered. A tool the server cannot offer, such as `web_search` without a Brave Search key, is still left out.
- **Confirmation switches** (`permissions.confirm`): with a switch on, a change of that kind is a proposal you apply or discard; with it off, the agent makes the change itself. See [Changes the AI makes](ai.md#changes-the-ai-makes).
- **Internet switches** (`permissions.web`): which web tools the agent is given at all. See [Internet switches](ai.md#internet-switches).
- **Note types** (`permissions.types`): the types the agent may read and change. Search, query, backlinks and asset results leave out notes of other types, reading one is refused with "This agent may not read <type> notes.", and every change to one is refused.
- **Outbox** (`permissions.outbox.send_without_review`, off by default): "Send from the Outbox without review". Off, what the agent drafts waits in the Outbox's Draft tab for you. On, its drafts are created ready and sent at once. See [Drafting into the Outbox](#drafting-into-the-outbox).
- **Browser automation** (`permissions.browser`, off by default): the browser box tools. See [Driving the browser box](#driving-the-browser-box).
- **Skills** (`permissions.skills`): **All skills**, or **Only these skills** from the vault's skills. See [Using skills](#using-skills).
- **Run wscript code** (`permissions.scripts.run_code`, off by default): the `run_script` tool, full access to the vault, the web and script secrets. See [Running wscript](#running-wscript).
- **Sub agents** (`permissions.subagents`, off by default): **Start sub agents**, how many run at once, and the agents it may delegate to. See [Sub agents](#sub-agents).

In a chat, the person or token running it still caps what the agent can change: an agent never writes more than its caller could.

## Agents that manage agents

An agent can read, create, change and delete other agents with four chat tools. They are ordinary tool names, so an agent has them only when its tools list allows them. An agent with no tools list, such as the Assistant, has them. On the agent screen's Permissions tab they are the "Agents" group.

| Tool | What it does |
|------|--------------|
| `list_agents` | Every agent's name, label, description, AI account, model, whether it is enabled and built in, and its trigger count |
| `get_agent` | One agent's file as stored, the agent parsed, and the agent-file vocabulary |
| `propose_agent_change` | Proposes a whole agent file: a change to an existing agent, or a new agent under a new name. The server checks it as `PUT /api/agents/{name}` would, without writing it |
| `propose_agent_delete` | Proposes deleting an agent. Its threads stay. The Assistant cannot be deleted |

The panel shows an agent proposal as a card with the WCL lines removed and added, and Apply and Discard. Apply checks the file is still the one the proposal was made against. If it changed since, the card says "This agent changed since the proposal" and nothing is written. Apply needs `admin`: from an app the server answers "Apply agent changes in the web app." Applied cards link to the agent's screen, and the sidebar's agent list refreshes.

The confirmation switch is "Ask before changing agents" (`permissions.confirm.agents`, on by default). With it off, the tool makes the change at once on the home server, whoever runs the turn with at least `write`, so in the desktop and Android apps too (see [Changes the AI makes](ai.md#changes-the-ai-makes)); a `read` caller gets the proposal. An unattended run with the switch on is not offered `propose_agent_change` or `propose_agent_delete`.

**No escalation.** An agent can never give another agent more than it has itself. A change is refused, with each problem named, when the agent it writes would have:

- a tool the proposing agent does not have. An agent with no tools list has every tool, so an agent with a tools list cannot write an agent without one;
- a note type outside the proposing agent's types. No types list means every type;
- a confirmation switch off that is on for the proposing agent;
- an Internet switch on that is off for the proposing agent;
- the Outbox switch `send_without_review` on when it is off for the proposing agent;
- Browser automation on when the proposing agent does not have it;
- a skill the proposing agent is not offered. No skills list means every skill, so an agent with a list cannot write an agent without one.

The whole new file is checked, not only what the change adds, so an agent with fewer permissions cannot edit a more powerful agent at all. An agent can never change or delete its own file: "An agent cannot change its own file." Deleting another agent needs only the tool and the switch. Each proposal records the agent that made it, and `POST /api/ai/apply` checks the change again against that agent as it is at the time of Apply, so an agent whose permissions were narrowed since cannot apply what it proposed before. Your own edits on the agent screen are never checked this way.

## Drafting into the Outbox

An agent drafts emails, SMS messages and calendar invites with three chat tools. Each creates one Outbox item and answers its id and status. They are ordinary tool names, so an agent has them only when its tools list allows them; the Assistant has them. On the agent screen's Permissions tab they are the "Outbox" group.

| Tool | What it drafts |
|------|----------------|
| `draft_email` | An email: `to`, `cc` and `bcc` (addresses or person note ids), `subject`, `body` as plain text, `attachments` (asset ids) and `in_reply_to` (the email note it answers, so it joins its thread) |
| `draft_sms` | An SMS for the phone to send: `to` (a number or a person note id), `body` and `device` (empty for the vault's one Android device with SMS sending on) |
| `draft_event` | A calendar invite: a new event, or with `event` a change to an existing event note. `title`, `when`, `location`, `attendees`, `description` and `calendar`; for a change an empty value keeps that part. Sent, Google emails the attendees |

A person note id resolves to the person's first email address or phone number when the item is drafted; the agent must be allowed to read person notes. Items are written as the agent, the Assistant included, so the Outbox rules hold:

- an item is created as a draft (`status` `draft`), and only an agent with "Send from the Outbox without review" on creates it ready (`ready`), so it is sent at once. The tool answers the status;
- an agent can change only its own drafts, and they stay drafts: it never marks an item ready or sends one, by these tools or by `propose_edit`. Its confirmation switches do not change this: with "Ask before creating and editing notes" off, an edit that marks an item ready is still refused.

On the Outbox page and the item's page an agent's draft carries an **AI** chip beside its status. Once sent, an item leaves the Outbox and its copy is a regular email, message or event note.

Drafting is not a change the chat asks you to apply, since the Outbox is where you check a draft, so the tools have no confirmation switch and unattended runs are offered them. They need a caller with `write`: a read-only token's chat gets an error. The chat shows each draft as a compact card, such as "Drafted an email to Ann Lee", linking to the item.

## Driving the browser box

With the browser box set up ([Browser box](install.md#browser-box)), an agent with **Browser automation** on can run commands in the box and drive its browser with playwright-cli while you watch the desktop. The switch is on the agent screen's Permissions tab, with the line "Lets this agent run commands in the browser box and drive a browser as you watch. Web pages can try to instruct it." It is off for every agent unless you turn it on, the Assistant included.

The switch is `permissions.browser = true` in the agent file, and it is apart from the tools list: an agent with no tools list (every tool) does not get the browser tools, and a tools list cannot name them. The tools are offered only when the server has a box (`VAULT_BROWSER_URL` set and not empty) and the switch is on.

| Tool | What it does |
|------|--------------|
| `browser_run {command, timeout}` | Runs a bash command in the box, in `/work`, killed after `timeout` seconds (1 to 300, 60 when `null`). The model gets the exit code, stdout and stderr, each cut to its last 20,000 characters, and whether it timed out. At most 4 commands run in the box at once; a fifth is an error result |
| `browser_file {path}` | Reads a file from the box, by an absolute path or one relative to `/work`. A PNG, JPEG, GIF or WebP image, such as a screenshot, goes to the model as an image, scaled as [Attachments](ai.md#attachments) says and counting toward the 8 images per question. Text, JSON, XML and YAML, such as a playwright-cli snapshot, are its text cut at 40,000 characters. Anything else is its path, media type and size |
| `browser_save {path, filename}` | Stores a file from the box in the vault as an asset, named `filename` or after the file, streamed to `.vault/incoming` as a download is |
| `browser_capture {what, target, url, full_page, session, filename}` | Captures from the open page into `/work/captures` and stores it as `browser_save` does. `what: "screenshot"` runs `playwright-cli screenshot`: the view, the whole scrollable page with `full_page`, or the element `target` names by snapshot ref or selector. `what: "image"` downloads an image with `curl`, from `url` or from the source (`currentSrc`, `src`, else `href`) of the element `target` names, read with `playwright-cli eval`; an image that is not on http or https, such as a `data:` one, is refused, with a screenshot of the element as the way round. `session` is the playwright-cli session the page is open in. The asset is named `filename`, else `screenshot-<time>.png` or `image-<time>` with the extension of its address |

`browser_save` and `browser_capture` follow the agent's **Ask before downloading files into the vault** switch (`permissions.confirm.downloads`). On, the panel shows a card with the path, file name, type and size, and Apply sends it to `POST /api/ai/apply`, which reads the file from the box and stores it; Apply is refused unless the agent that proposed it still has Browser automation. Off, and with `write`, the file is stored at once. An unattended run (see [What an unattended run may do](#what-an-unattended-run-may-do)) gets `browser_run` and `browser_file` when the agent has the switch, and `browser_save` and `browser_capture` only when its downloads switch is off.

The panel shows a `browser_run` step as the command, with its output in the step's collapsible body, and a `browser_file` image as a small thumbnail. The image is not in the vault, so the step's summary carries the thumbnail as a JPEG data URL of at most 240 pixels; it is kept in the thread with the step, and the model's own copy is held for that question only.

Each `browser_run` writes one line, to the run's log for an unattended run and to the server log, and the step, with its command and output, is kept in the thread:

```text
browser: exit=0 ms=812 timed_out=false cmd=playwright-cli snapshot
```

`cmd` is the first 120 characters of the command, on one line. A run the box refused or could not take writes `exit=-` and an `error="..."`.

Pages and command output are untrusted: a page can try to instruct the agent. The box holds no vault credentials, but whatever you sign in to in its browser is reachable by every agent with the switch.

Agent files that hold `browser = true` are new: an app from before the browser box refuses an agent file holding it, as it does `permissions.outbox`. The file holds `browser` only when it is on, so turning the switch off makes the file readable by older apps again. A release ships the apps first (see [Releasing a format change](android-release.md#releasing-a-format-change)).

## Using skills

Skills are folders of instructions and files in the vault's `skills/` folder, in the open Agent Skills format; [Agent skills](skills.md) covers writing and importing them. An agent is offered the valid skills its **Skills** permission allows: every valid skill (no `skills` list in the file, the default) or only those named (`permissions.skills = ["pdf-forms"]`). A skill with errors is offered to no agent. `[]` offers none, and the agent gets no skill tools.

The agent's system text ends with an "Available skills" section, after its prompt, its memories and the open note: each offered skill's name and description, and a line telling it to call `use_skill` before following one. When the agent has the browser box, the line says how to run a skill's scripts there; otherwise it says scripts can be read but not run.

The list is a snapshot kept per thread, in the thread's transcript: taken on the thread's first turn and sent unchanged on every later turn, so adding or changing skills never changes a running conversation's instructions, and prompt caching holds. To give a thread the current list, type `/update-skills` alone in the chat box, in the panel or the agent screen, in any client. The turn sends no message of yours: the server takes a fresh snapshot, tells the model "The available skills changed; the list at the end of your instructions is current.", and the conversation shows a small "Skills updated (<n>)" line instead of your bubble. Anything else starting with `/` is sent as typed. A thread from before skills takes its snapshot on its next turn; a conversation with no thread lists the skills as they are on each turn.

| Tool | What it does |
|------|--------------|
| `use_skill {name}` | Activates a skill: answers its `SKILL.md` instructions, its file list with sizes and media types, its description, license, compatibility and metadata, and its `allowed-tools` with a note that they grant nothing |
| `read_skill_file {name, path}` | Reads one file of the skill, its path from the skill's folder. Text, JSON, YAML, TOML and XML are its text cut at 40,000 characters; a PNG, JPEG, GIF or WebP image goes to the model as an image, scaled as [Attachments](ai.md#attachments) says and counting toward the 8 images per question; anything else is its path, media type and size. A path outside the skill, absolute or with `..`, is refused |
| `install_skill_in_browser {name}` | Copies the skill's folder into the browser box at `/work/skills/<name>`, replacing what was there, through the box's exec service, file by file. Files starting with `#!` are made executable. The agent then runs its scripts with `browser_run` |

`use_skill` and `read_skill_file` are ordinary tool names, the "Skills" group on the Permissions tab, and only read, so an unattended run is offered them. `install_skill_in_browser` comes with the browser tools: only with Browser automation on, a browser box, and skills offered. A skill not offered to the agent is refused by every tool. A skill's `allowed-tools` is shown and recorded but grants nothing: the agent's own permissions decide its tools. Each call is an `ai.skill` entry in the [activity log](activity.md).

## Agents that manage skills

An agent can also read, create, change and delete the skills themselves, with six chat tools. They are ordinary tool names, so an agent has them only when its tools list allows them; an agent with no tools list, such as the Assistant, has them. On the Permissions tab they are the "Skill management" group.

| Tool | What it does |
|------|--------------|
| `list_skills {}` | Every skill in the vault, offered to the agent or not: its name, description, license, compatibility, metadata, `allowed-tools`, file count, size, whether it is valid and its issues, and `offered`, whether `use_skill` opens it for this agent |
| `get_skill {name}` | One skill: its `SKILL.md` as stored, the frontmatter fields, the body, the file list with sizes and media types, its issues, and a summary of the skill format |
| `propose_skill_change {name, skill_md}` | Proposes a whole `SKILL.md`: a new skill under a new name, or the new text of an existing one. It is checked as `PUT /api/skills/{name}` checks it, without writing: an invalid one is refused with its issues |
| `propose_skill_file_change {name, path, text, base64}` | Proposes one other file of a skill, with exactly one of `text` or `base64` (the other `null`). The path is from the skill's folder, normalised and kept inside it, and the skill's limits apply. `SKILL.md` is refused: it goes through `propose_skill_change`. A file for a skill that does not exist yet can be proposed, but applies only once the skill's `SKILL.md` is applied |
| `propose_skill_file_delete {name, path}` | Proposes deleting a file, or every file in a folder. `SKILL.md` cannot be deleted alone |
| `propose_skill_delete {name}` | Proposes deleting the whole skill |

The panel shows a skill proposal as a card: the `SKILL.md` or file lines removed and added, or for a binary file its size and type, with Apply and Discard, and **Apply all** includes it. Apply checks that what the proposal replaces (the `SKILL.md`, the file, the folder's files, or for a delete the whole skill) is unchanged since; if it changed, the card says "This skill changed since the proposal" and nothing is written. Apply writes through the same calls as the `/api/skills` routes, so each write is a `skill.change` entry in the activity log, and the proposal and its apply are `ai.proposal` and `ai.applied` entries. Apply needs `admin`: from an app the server answers "Apply skill changes in the web app." Applied cards link to the skill's screen, and the Skills page and the sidebar's count refresh. The next message tells the model what was applied, such as `[Applied by the user: created skill pdf-tools]`.

The confirmation switch is "Ask before changing skills" (`permissions.confirm.skills`, on by default, written to the agent file only when off). With it off, the tool makes the change at once on the home server, whoever runs the turn with at least `write`, so in the desktop and Android apps too, and the panel shows it as made; a `read` caller gets the proposal. An unattended run with the switch on is not offered the four propose tools. An agent cannot give another agent the switch off when it is on for itself (see [No escalation](#agents-that-manage-agents)).

Once a skill exists, the agent that created it may call `use_skill` on it at once when its **Skills** permission offers it. The thread's "Available skills" list does not change: type `/update-skills` to give the thread the new list.

## Running wscript

Agents run wscript three ways (script-tools T1 to T6). The built-in `wscript` skill teaches the language and the vault API; an agent that can run scripts is told to open it first.

- **Its own code, with `run_script {code, function, args}`.** Offered only when the agent file sets `permissions { scripts { run_code = true } }`, the agent screen's **Run wscript code** switch. `code` is a whole wscript unit; `function` (default `main`) is called with `args`, JSON text of an array, each element a `Value`. It answers the return value as JSON, the log, the time taken and the instructions used, or the compile or run-time error to fix.
- **A script tool of the vault**, `tools/<name>.wcl` with its script (see [Tool files](vault-format.md#tool-files)), offered under its name when it is valid and the agent's tools list, if it has one, names it. Settings > Tools lists, edits and runs them, and shows why an invalid one is not offered.
- **A skill's script tool**, `skills/<skill>/tools/<tool>.wcl` with the skill's own `scripts/<script>.ws`, offered as `<skill>__<tool>` from the round after the agent calls `use_skill` on the skill, for the rest of the thread.

Safety:

- **Run wscript code is full access.** The code reaches every `vault` function on every note type (the agent's `permissions.types` do not limit it), HTTP to any host, the `ai` module, and every script secret by name, for up to five minutes and 50 million instructions. Its changes are made at once, as the agent, with no proposal. Only an admin grants it: agent files need `admin`, an agent may propose it for another agent only when it has it itself, an agent never changes its own file, and such a change always waits for an admin's Apply, whatever the agent's "Ask before changing agents" switch says; a `write` token from an app gets "Only an admin turns on Run wscript code".
- **A tool's author decides what it can reach.** A tool runs with the modules a task has, but only the hosts in its `allow_domains` and the secrets in its `secrets`, with no `fs` directories, within its `timeout`. Its changes are made at once as the calling agent: the agent's confirmation switches do not turn them into proposals. Writing a tool needs `admin`; an agent proposes one with `propose_tool_change {name, wcl, script_source}` (the tool file and, when given, its script, in one commit) or `propose_tool_delete {name}` (the script goes too when nothing else names it), under "Ask before changing script tools" (`permissions.confirm.tools`, on by default). With the switch off, the change is made at once on the home server, whoever runs the turn with at least `write`; a `read` caller gets the proposal. `list_tools` and `get_tool` read.
- **Every run is logged with its version.** Each run is an `ai.script` entry in the [activity log](activity.md): the agent, the thread and turn, the tool or `run_script`, the outcome and error kind, the time, the instructions, the start of the log, and the script that ran: for a vault tool the SHA-256 of its script and of its tool file and the vault generation they were read at, for a skill's tool the skill, the script's path and its hash, and for `run_script` the hash of the code, whose whole text stays in the thread as the tool call's input.

A script's changes are made at once, so a turn run by a `read` caller is offered no script: neither `run_script` nor any script tool. A `write` caller's turn, an app's, has the agent's own scope and is offered them. Scripts run on the home server only. The apps send their turns there, so nothing runs on a device, and they leave Settings > Tools to the web app.

## Sub agents

An agent can break a task down and hand the parts to sub agents with the `start_subagent {task, instructions, agent, permissions, model}` tool (subagents U1 to U8). It is offered only when the agent file sets `permissions { subagents { allow = true } }`, the Permissions tab's **Start sub agents** switch, and only in a thread.

- **Throwaway or named.** With `agent` null the sub agent is a throwaway: the agent itself, its prompt ending with a "Sub agent" section holding the task and `instructions`. It reads the agent's memories but writes none. With `agent` naming one of the agents `subagents.agents` lists (the Permissions tab's **Agents it may delegate to**), that agent does the task with its own prompt and memories. Any other name is refused.
- **At once, one level deep.** The calls of one round run at the same time, at most `subagents.max` of them (1 to 8, 4 by default); a call past it fails, saying so. A sub agent is never offered `start_subagent`, whatever its own file says. It has its own limit of 50 rounds, and its conversation compacts like any thread's.
- **Never more than the agent.** A throwaway has the agent's permissions; `permissions` may only narrow them: a tools or types list keeps only what the agent has, a confirmation switch only turns on, an Internet switch, browser automation or `run_code` only turns off. Anything wider is ignored and the result says so. A named agent gets what both agents have: a tool, type, skill, Internet switch, browser automation or `run_code` only when both have it, and each confirmation switch on when either has it on. It runs on the agent's AI account and model, unless `model` names another model of that account, or the named agent names an account of its own.
- **Its changes come back to you.** A change the sub agent makes without asking is authored `agent:<agent>/sub:<n>` for a throwaway, `agent:<name>` for a named agent. A proposal appears in the chat that started it, under the sub agent's block, with Apply and Discard there; the sub agent does not wait for you, and its answer lists what it proposed. A run with nobody to ask, from a trigger or a call, gives its sub agents only the change tools whose switch is off, as it has itself.
- **The result is its answer.** The call's result is the sub agent's last message, with a status line: its rounds, its tool calls and how it ended.

In the chat each call is a block, "Sub agent: <task>" or "Delegated to <agent>: <task>", with its status (running, with the tool it uses and its round; done; failed; stopped), the time it took, and its result to expand once done. **Stop** on the block stops that sub agent alone, and the agent is told you stopped it. Clicking the block's title opens the sub agent's session in a pane beside the chat: its messages, tool calls and results, live while it runs, read only, with Stop.

A session is a thread of origin `subagent:<thread>`, kept with the thread that started it: the session picker, the panel's thread picker and the thread lists leave it out, and it is found only from its block. Deleting, archiving or unarchiving the thread does the same to its sessions, editing a message before the call deletes the call's session, and a fork keeps the blocks but not the sessions. This holds wherever the thread changes: in the web app, in the desktop and Android apps, which change their local copy and sync, and on the home server when such a change arrives from an app that left the sessions out. Each run is an `ai.subagent.start` and an `ai.subagent.end` entry in the [activity log](activity.md), with the parent thread and turn, the session, the agent or throwaway, the rounds, the outcome and the time; the sub agent's own turns log under its session.

Turning sub agents on, or adding an agent to delegate to, is guarded like Run wscript code: an agent may propose it for another agent only when it has it itself, and such a change always waits for an admin's Apply ("Only an admin grants sub agents" from an app).

## Threads

Every conversation with an agent is a note of the built-in type `agent_thread`, holding the agent's name, where it came from (`user`, `trigger:<name>` for a trigger run, or `mention` for a call from a note), when it was last active, the transcript and a readable copy of the conversation. The server writes the thread as each turn streams, so closing a tab loses nothing. Proposals and what became of them are kept in the thread, so a reopened thread shows its pending proposals with Apply and Discard. [Threads](ai.md#threads) describes them in full.

The note tools leave threads out unless the agent's `permissions.types` names `agent_thread`, so an agent does not read chats back as vault content. Deleting an agent keeps its threads.

The chat panel is another view of the same threads. Under its agent picker, a thread picker lists the chosen agent's threads, newest activity first and archived ones left out, with the running spinner and the "New reply" dot. **New thread**, at the top of the list, starts another, and **Open in agent screen** opens the shown thread in the agent screen. A thread open in the panel and in the agent screen at the same time is one conversation: the answer streams in both, and sending, Stop, Apply and Discard in either show in both. Each view chooses its thread by itself, so choosing a thread in one never changes the other. The panel remembers its agent and thread in this browser and opens on them again, or on a new thread when that one was archived or deleted. The panel still sends the open note with each message, whichever thread it shows; the agent screen sends none. The desktop and Android apps work the same way.

In the agent screen, **Rename**, **Archive** (or **Unarchive**) and **Delete** in the bar act on the chosen thread. These are ordinary note changes, authored by you: Delete asks first, then moves the thread note to the trash, where it can be restored. Archived threads are hidden from the session picker until **Show archived**, at its foot, lists them dimmed. An archived thread still opens, and sending a message in it takes it out of the archive. When the chat panel's thread is archived or deleted elsewhere, its next message starts a new thread.

A turn runs on the server until it ends or you press **Stop**, whether or not anything shows it. Opening another thread, choosing another agent in the panel, closing the panel or the tab only stops showing it; the turn goes on writing to its thread. A running thread shows a small spinner beside its title in the agent screen's session picker, and the agent shows one in the sidebar; opening the thread shows the answer so far and follows the rest, with the activity line counting from when the turn started. A thread that answered while it was not open shows a dot, "New reply", until you open it. One thread runs one turn at a time ("Still answering" while it does), and an agent runs at most 4 turns at once across its threads, trigger runs included. [The turns form](ai.md#the-turns-form) describes the routes.

## Memory

An agent keeps its own memories from one conversation to the next. Each memory is a note of the built-in type `agent_memory`, holding a short `title`, the agent's name in `agent`, and the memory's text in the content field `body`. Memory notes sync to the apps and keep their history like any note. Deleting an agent keeps its memories.

Four chat tools reach only the agent's own memories:

| Tool | Does |
|---|---|
| `remember {title, text}` | Creates a memory and answers its id |
| `recall {query, limit}` | The agent's memories whose title or text holds every word of `query`, ignoring case, or all of them for an empty query, newest first, with their full text. `limit` is 1 to 50, 20 when `null` |
| `update_memory {id, title, text}` | Changes a memory's title, text or both; `null` keeps a part. A memory of another agent's is refused |
| `forget {id}` | Moves a memory to the trash, where you can restore it. A memory of another agent's is refused |

They are ordinary tool names, so an agent has them only when its tools list allows them; on the Permissions tab they are the "Memory" group. They change nothing but the agent's own memory, so they need no confirmation, and a trigger run is offered them whatever its confirmation switches say. Their changes are authored by the agent, so they never fire a trigger. The person or token running a chat must be able to write notes for the agent to change its memory.

Every turn's instructions gain a "Your memories" section after the agent's prompt, so the prompt itself stays the same from turn to turn for the provider's prompt caching. The section lists the agent's memories, newest first, one line each: `- <title> (<id>): <the first 200 characters of its text>`. The lines stop at 8,000 characters, with a last line saying how many more memories there are and that `recall` reads them. An agent with no memories gets no section.

The note tools leave memories out unless the agent's `permissions.types` names `agent_memory`, as they do threads.

| Limit | Value | Past it |
|---|---|---|
| A memory's title | 120 characters | `remember` and `update_memory` are refused |
| A memory's text | 8,000 characters | `remember` and `update_memory` are refused |
| Memories one agent holds | 500 | `remember` is refused, asking the agent to forget some or merge several into one |

### The Memory tab

The **Memory** tab of the agent's settings lists the agent's memories, newest first, with a search box that keeps those whose title or text holds every word typed. **Add memory** opens an empty title and text; **Edit** opens a memory's title and text, saved together with one **Save**; **Delete** asks first, then moves the memory to the trash, where it can be restored. These changes are authored by you, and the same limits apply. The tab works in the desktop and Android apps too: the changes are note writes to the vault on the device, synced like any note.

The tab uses these routes. Reading needs `read`; the changes need `write`.

| Route | Does |
|---|---|
| `GET /api/agents/{name}/memories?query=` | `{memories: [{id, agent, title, text, version, created, updated}], held, limit}`, newest first; `query` keeps those whose title or text holds every word |
| `POST /api/agents/{name}/memories` | A new memory from `{title, text}`. 400 `invalid_memory` past a limit, 409 `memory_full` at 500 memories |
| `PATCH /api/agents/{name}/memories/{id}` | Changes the title, the text or both from `{title?, text?, base_version?}`; with `base_version`, 409 when the memory changed since |
| `DELETE /api/agents/{name}/memories/{id}` | Moves the memory to the trash |

### Other agents' memories

An agent that manages agents can reach other agents' memories with four more tools, in the "Agents" group of the Permissions tab:

| Tool | Does |
|---|---|
| `list_agent_memories {agent, query}` | The other agent's memories whose title or text holds every word of `query`, or all of them, newest first, each with its id, title, the first 200 characters of its text and when it changed |
| `get_agent_memory {id}` | One memory in full |
| `propose_agent_memory_change {agent, id, title, text}` | Proposes a new memory when `id` is `null`, else a change to memory `id`; `null` keeps a title or text as it is |
| `propose_agent_memory_delete {id}` | Proposes moving a memory to the trash |

A memory steers the agent that reads it, so these follow the no-escalation rule of [Agents that manage agents](#agents-that-manage-agents): an agent reaches another agent's memories, to read them as well as to change them, only when it could change that agent's file. It must have `propose_agent_change`, and the other agent must have nothing it lacks. Otherwise each tool is refused, naming the problems. An agent cannot use these tools on its own memories; it uses its memory tools for those.

The proposals follow "Ask before changing agents" (`permissions.confirm.agents`). With it on, the panel shows a memory card with the title and text before and after, and Apply and Discard; Apply checks the memory is unchanged since the proposal ("This memory changed since the proposal") and checks the rule again against the proposing agent as it is then. Applying needs only `write`, since memories are notes, so it works from the apps too. With the switch off, the tool makes the change at once. An unattended run with the switch on is not offered the two propose tools. The changes are authored by the proposing agent, so they never fire a trigger.

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

A run starts once the note has been quiet for 30 seconds: several changes to one note within that time make one run, with the last change. A condition that no longer matches by then, or a note gone by then, starts nothing. A trigger with no `type` does not fire for `agent_thread` or `agent_memory` notes; name the type to watch threads or memories.

Note events fire on the home server only, after the change is written. A change made in the desktop or Android app fires when it reaches the server by sync. The apps themselves never run triggers.

### Calls from notes

Every enabled agent can be called from a note. Type `@` in the editor and choose **Ask <label>**, which the picker lists before the notes for each agent the note's type may call. The picker becomes a request box, "Ask <label>…": write what the agent should do and press Enter. Shift+Enter starts a new line; Escape, or Enter with nothing written, leaves nothing in the note. An agent with no mention trigger answers a call from any note type except its own threads. A trigger with `mention = true` adds to that: `type` limits which note types may call the agent, and `prompt`, which is optional here, is added before the request. With mention triggers that name types, only those types may call it.

```wcl
trigger asked {
  mention = true
  type    = ["memo", "project"]
  prompt  = "Keep your answer short."
}
```

Enter commits the call into the note as one link, with an id the editor makes for it, and the note's next save carries it whole:

```text
[@Clerk list what to buy for the gutters](agent:clerk?call=01JB8Z4K2MQ7V3X9T5R6W8Y2ZB)
```

The editor and the note page show a call as a chip: the agent's icon, `@<label>` and the request, with a spinner while its run goes. Clicking it opens the run's thread. Until the server has seen the call the chip says "Sending…"; a call the server did not run, because the agent may not be called from that note's type, is disabled or is missing, says so. The chip's text stays editable like a link's.

Only a committed call calls. Plain text such as `@clerk tidy this` calls no one, and neither does a call in inline code. A call stands in the text of a note's content: paragraphs, headings, list items, quotes, toggles and admonition titles.

When the server saves a note, it reads the note's calls at once. Each call id it has not seen before is recorded and answered once:

1. It opens a thread, with origin `mention`, titled with the note's title and the start of the request.
2. It starts the run. Its first message is the trigger's prompt, then the request, then the paragraph or list item around the call, then a line naming the note:

```text
Keep your answer short.
list what to buy for the gutters
Context: Roof work: @Clerk list what to buy for the gutters
Called from note 01JB8Z0Q6W4ZK3V9T1M5XH7N2C (type memo, title "Roof").
```

The `Context:` line is left out when the call is all its paragraph holds. The server never changes the note for a call. Saving the note again, editing the call's text or moving it never calls again, and neither does the same call copied into another note; delete the call and commit a new one to ask again. A call to an agent that may not be called from the note, is disabled or missing, or has reached its hourly limit is recorded as not run and is never tried again.

The run is unattended, as any trigger run is (below). To let it change the note it was called from, turn off **Ask before creating and editing notes** on the agent's Permissions tab; the Triggers tab warns when the agent could not. Otherwise it answers in the thread, which the chip opens.

A call written by an agent, including the called agent's own changes to the note, calls no one. Calls are answered on the home server only: a call written in the desktop or Android app is answered when the note reaches the server by sync.

Links that earlier versions wrote in place of a call, `[@Clerk](/agents/clerk/chat?thread=<id>)`, stay ordinary links.

### What an unattended run may do

A trigger run has no one present to apply a proposal, so it is offered only:

- the read tools the agent's permissions allow,
- the change tools whose confirmation switch is off for the agent, and
- the Outbox draft tools, since what they draft waits in the Outbox for you.

A change tool whose switch is on is left out of the run's tool list, so nothing is left pending. The Triggers tab lists the tools a run is offered. A run acts as the agent itself: with `write` for notes, and `admin` for types, taxonomies, agents and skills only when its permissions allow those tools. Its permissions are its only limit, as no person or token is running it.

## No loops

Every change an agent makes is authored by the agent (author kind `agent`) in the note's history: changes its tools make, changes you apply from its proposals, and its writes to its threads. A change authored by any agent never fires a note event trigger and is never read for calls, so an agent cannot set itself or another agent off.

Changes by you, by tokens, by scripts, tasks and handlers, and by sync from the apps do fire triggers. A handler that runs because of an agent's change writes as the handler, so it can fire a trigger.

## Limits

| Limit | Value | Past it |
|---|---|---|
| Runs of one agent at once | 2 | Further runs wait their turn |
| Runs waiting for one agent | 50 | The oldest waiting run is dropped, and the server log says so |
| Trigger runs one agent starts in an hour | 60 | Further runs are skipped and logged, with one notification an hour |
| Model calls in one run | 12 | The run stops, as a chat turn does |
| Quiet time before a note event run | 30 seconds | Changes to the same note coalesce |

## Runs and notifications

Every trigger run is recorded in `state.db` (`agent_runs`): the agent, the trigger, the thread, when it started and ended, its outcome and its message, and its [trail lines](ai.md#the-trail-line). The server log has the trail lines and one line per run:

```text
agent run 12: agent=triage trigger=new_email thread=01JB8Z1A2B3C4D5E6F7G8H9J0K outcome=succeeded
```

The Triggers tab shows each trigger's last 20 runs, with a link to each run's thread. A run that fails, through a provider error, a refusal or a turn that does not end, leaves one bell notification, "<Agent>'s <trigger> run failed", that opens its thread. A run that goes well sends none.

## API

| Method and path | Scope | Does |
|---|---|---|
| `GET /api/agents` | read | Every agent's summary, Assistant first, and the tools `permissions.tools` may name. A summary's `mention` is `null` for a disabled agent, else `{ types }`: the types whose notes may call it, `null` for every type (also for an agent with no mention trigger) |
| `GET /api/agents/{name}` | read | The parsed agent and its WCL |
| `PUT /api/agents/{name}` | admin | Writes the agent from WCL, `{ wcl }`, or the JSON form `GET` answers |
| `DELETE /api/agents/{name}` | admin | Deletes the agent file; its threads stay |
| `GET /api/agents/calls/{call}` | read | What became of a call committed in a note: `{ agent, note, thread, running }`, `thread` null for a call that was not run; 404 while the server has not seen the call |
| `GET /api/agents/{name}/runs` | read | `{ tools, triggers: [{ name, kind, runs }] }`: the tools a trigger run is offered, and each trigger's last 20 runs, newest first, each `{ id, thread, started, ended, outcome, message }` with times in milliseconds |
| `GET /api/agents/{name}/threads` | read | The agent's threads, newest activity first, each with `archived`; archived threads only with `?archived=true` |
| `POST /api/agents/{name}/threads` | read | Starts a thread from `{ title? }` |
| `GET /api/agents/{name}/threads/{id}` | read | A thread with its messages, steps and proposals |
| `PATCH /api/agents/{name}/threads/{id}` | write | Renames a thread from `{ title }` |
| `DELETE /api/agents/{name}/threads/{id}` | write | Moves a thread to the trash |

The chat routes take `agent` and `thread`; see [The chat route](ai.md#the-chat-route). In the apps, the runs, calls and threads routes are answered by the home server.
