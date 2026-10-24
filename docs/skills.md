# Agent skills

A skill is a folder of instructions, and any scripts, references and assets they need, that an agent can follow when a task calls for it. TheVault keeps skills in the open [Agent Skills format](https://agentskills.io/specification): a folder holding `SKILL.md`, whose YAML frontmatter names and describes the skill, followed by the Markdown instructions.

## Where skills live

Each skill is a folder in the vault, `skills/<name>/`, holding `SKILL.md` and any other files exactly as they were written: Markdown, scripts, references and binary assets. The folder name is the skill's name, and it must equal the `name` in the frontmatter. The format's rules and how the vault checks them are in [Skills](vault-format.md#skills) in the vault format.

```markdown
---
name: pdf-forms
description: Fills in PDF forms. Use when a PDF form arrives or the user asks to fill one.
license: MIT
metadata:
  author: me
  version: "1.0"
---

# PDF forms

Run `scripts/fill.py` with the form's path.
```

The server checks `SKILL.md` every time it lists or writes a skill:

- `name`: 1 to 64 characters of lowercase `a-z`, `0-9` and hyphens, not starting or ending with a hyphen, no two hyphens in a row, and the same as the folder name. `import` is reserved.
- `description`: required, 1 to 1024 characters.
- `compatibility`: optional, 1 to 500 characters.
- `metadata`: optional, string keys to string values. Quote a value such as `"1.0"`, which YAML would otherwise read as a number.
- `license` and `allowed-tools`: optional strings.
- Any other key is kept and shown as a warning. A warning leaves the skill valid.

A skill holds at most 200 files and 20 MiB, and no symlinks. A write that would break a limit is refused, and a skill on disk that breaks one is listed as invalid. A file an agent reads as text is at most 2 MiB.

Backups include `skills/` like every other vault folder. The desktop and Android apps receive skills when they sync, binary files too. Skills need apps that know the vault format version 3: publish the new app builds with the server (see [Releasing a format change](android-release.md#releasing-a-format-change)).

## Import and export

An import takes a zip in one of two shapes:

- One top-level folder holding `SKILL.md`, such as `pdf-forms/SKILL.md` and `pdf-forms/scripts/fill.py`.
- `SKILL.md` at the zip's root, beside the skill's other files.

The skill is named by the frontmatter `name`, whatever the folder in the zip is called. `__MACOSX/` entries and `.DS_Store` files are skipped. The whole zip is checked before anything is written: a zip without `SKILL.md`, with more than one top-level folder, with a symlink or a path that leaves the folder, over a limit, or whose `SKILL.md` has an error writes nothing. An import of a name that exists is refused unless it replaces the skill, and a replace removes the files the zip does not hold.

An export downloads `<name>.zip`, one folder `<name>/` holding the skill's files, which imports again unchanged.

## The API

| Endpoint | Scope | What it does |
|---|---|---|
| `GET /api/skills` | `read` | `{ "skills": [...] }`, each `{ name, description, license, compatibility, metadata, allowed_tools, extra, files, file_count, size, valid, issues }`. `files` and `file_count` are the number of files, `size` their bytes, `extra` the frontmatter keys the format does not name, and each issue is `{ at, message, severity }` |
| `GET /api/skills/{name}` | `read` | The same, with `body`, the Markdown after the frontmatter, and `files` as a list of `{ path, size, media_type, text }`, where `text` says whether the file reads as UTF-8 text |
| `GET /api/skills/{name}/files/{path}` | `read` | The file's bytes |
| `GET /api/skills/{name}/export` | `read` | The skill as a zip |
| `PUT /api/skills/{name}` | `admin` | The whole `SKILL.md` as the body. 201 when it creates the skill, 200 when it replaces `SKILL.md`; 400 `invalid` with the issues when it fails, writing nothing |
| `PUT /api/skills/{name}/files/{path}` | `admin` | The body as the file's bytes, in an existing skill. Answers `{ name, file }` |
| `DELETE /api/skills/{name}/files/{path}` | `admin` | The file, or every file in the folder `path`. `SKILL.md` cannot be deleted alone. Answers `{ name, deleted }`, the paths removed |
| `DELETE /api/skills/{name}` | `admin` | The whole folder |
| `POST /api/skills/import` | `admin` | A zip, as the body or as the file field `file` of a `multipart/form-data` form. `?replace=true` replaces an existing skill; without it an existing name is 409 `skill_exists` with the skill's `name`. 201 for a new skill, 200 for a replaced one, each with the skill and `replaced` |

A broken limit is 413 `too_large`. Every write is recorded in the activity log as a `skill.change` entry.

## Skills in the web app

The left sidebar's **Agents** section ends with one **Skills** item, after the agents. Its number is how many skills the vault holds, and a warning mark beside it means at least one skill fails the format's checks. Click it to open the Skills page, `/skills`, in the active pane.

The Skills page lists every skill by name. Each one shows its description (the first two lines), its number of files and their size, and its license if it has one. A skill that fails the format's checks is marked **Invalid** with its first problem; open it to see them all. An invalid skill is offered to no agent until it is fixed. Type in the search box to list only the skills whose name, description or license holds every word you type. Click a skill's name to open its skill screen.

Two buttons in the page's header add skills:

- **New skill** asks for a name, creates the skill with a `SKILL.md` holding that name and a placeholder description, and opens it. The name becomes lower case, with a hyphen for each run of other characters, as the format requires. Replace the placeholder description before you save: agents choose a skill by its description.
- **Import** offers two choices:
  - **From a zip** takes a zip holding one top-level folder, or `SKILL.md` at the zip's root. The skill is named by its frontmatter `name`. Nothing is written if the skill fails the checks. If a skill with that name exists, you are asked whether to replace it. The imported skill opens.
  - **From a folder** takes a skill folder from your computer, with the same checks. It appears only in browsers that can choose a folder.

With no skills yet, the page explains what a skill is and links to this page.

The command palette has **New skill**, which opens the Skills page with its name box shown, and **Open skill** followed by the skill's name.

## The skill screen

Each skill opens at `/skills/<name>`, in a pane named "Skill".

| Tab | Holds |
|---|---|
| Instructions | The frontmatter as a form and the Markdown instructions |
| Files | The skill's folder as a tree |

### Instructions

The form edits the frontmatter keys the format names:

- **Name**: read only. It is the skill's folder name. To rename a skill, export it, change `name` in `SKILL.md`, and import it again.
- **Description**: what the skill does and when to use it, at most 1024 characters. The count under the box turns red past the limit.
- **License** and **Compatibility** (at most 500 characters): optional.
- **Metadata**: key and value rows. **Add metadata** adds a row. A row with an empty key is left out.
- **Allowed tools**: the format's space-separated `allowed-tools`. TheVault records it but grants nothing from it: each agent's own permissions decide its tools.

Below the form, the instructions are in a monospace editor. **Preview** shows them rendered as Markdown; **Edit** returns to the editor.

**Save** writes the whole `SKILL.md`. Frontmatter keys the form does not show are kept as they were. The server checks the file before writing it. Each problem it finds shows under the field it concerns, and any other problems show above the form. A warning, such as a frontmatter key the format does not name, leaves the skill valid.

### Files

The tree shows `SKILL.md` first, then the folders, then the other files, each with its size.

- Click a text file to open it in an editor, change it, and save it with **Save**. Saving `SKILL.md` here runs the same checks as the Instructions tab.
- A file that is not text, or is larger than 2 MiB, shows its size and type with **Download**.
- **Upload files** writes the chosen files into the folder picked in **Folder**. You are asked before a file of the same name is replaced.
- **New file** creates an empty file in that folder and opens it.
- **New folder** adds a folder to the tree and picks it in **Folder**. The folder is kept once a file is in it.
- The pencil beside a file or folder renames it. A name is a path inside the skill's folder: `scripts/run.sh` moves a file into `scripts`, and a path with `..` is refused.
- The bin beside a file or folder deletes it, after you confirm. Deleting a folder deletes every file in it.

`SKILL.md` cannot be renamed or deleted.

### Export and Delete

The **⋯** menu beside the skill's name holds:

- **Export**: downloads the skill's folder as `<name>.zip`, ready to import into another vault or tool.
- **Delete skill**: deletes the skill's folder after you confirm, and returns to the Skills page. No agent is offered it afterwards.

## How agents use skills

An agent is offered every valid skill, or only those its Permissions tab's **Skills** section names. Each turn's instructions end with an "Available skills" list of the offered skills' names and descriptions, so write the description to say what the skill does and when to use it: that is all the agent sees until it chooses the skill. Choosing it, the agent calls `use_skill` and reads the instructions and the file list, then reads the files the instructions point to with `read_skill_file`, by their paths in the skill.

A conversation keeps the list it started with. After you add or change skills, type `/update-skills` in the chat box to give the open thread the current list; the conversation shows "Skills updated (<n>)".

Scripts in a skill run only in the browser box: an agent with Browser automation copies the skill to `/work/skills/<name>` with `install_skill_in_browser` and runs them there with `browser_run`. Without the box, an agent can read the scripts but not run them. `allowed-tools` never grants a tool. See [Using skills](ai-agents.md#using-skills) for the details.

## Agents that create skills

An agent with the skill management tools can write skills too: `list_skills` and `get_skill` read them, and `propose_skill_change`, `propose_skill_file_change`, `propose_skill_file_delete` and `propose_skill_delete` propose a new `SKILL.md`, a change to one, a file added or changed (as text or base64), a file or folder deleted, or a whole skill deleted. Every proposal is checked as the API's writes check it, so an invalid `SKILL.md`, a path outside the skill or a broken limit is refused before you see it. The chat panel shows each as a card with the lines added and removed, and you apply or discard it. Applying needs `admin`, so it works in the web app only.

Turn off **Ask before changing skills** on an agent's Permissions tab (`permissions.confirm.skills = false`) to let it make these changes without asking. Once a skill exists, the agent may use it at once when its **Skills** permission offers it; type `/update-skills` to add it to the thread's list. See [Agents that manage skills](ai-agents.md#agents-that-manage-skills).

## In the apps

The desktop and Android apps have the **Skills** item, the Skills page and the skill screen, but only show skills: there is no **New skill**, no **Import**, no Save and no file changes, and the screen says "Edit skills in the web app". Changing a skill needs `admin`, which an app's device token never has. **Export** and **Download** work in the apps.
