# Agent skills

A skill is a folder of instructions, and any scripts, references and assets they need, that an agent can follow when a task calls for it. TheVault keeps skills in the open [Agent Skills format](https://agentskills.io/specification): a folder holding `SKILL.md`, whose YAML frontmatter names and describes the skill, followed by the Markdown instructions.

## Skills in the web app

The left sidebar's **Agents** section lists your skills under **Skills**, beneath the agents. Each skill shows by its name. A skill that fails the format's checks shows a warning mark; hover over it to read why. An invalid skill is offered to no agent until it is fixed.

Use the **+** beside **Skills** to add one:

- **New skill** asks for a name and creates the skill with a `SKILL.md` holding that name and a placeholder description. The name becomes lower case, with a hyphen for each run of other characters, as the format requires. Replace the placeholder description before you save: agents choose a skill by its description.
- **Import skill from a zip** takes a zip holding one top-level folder, or `SKILL.md` at the zip's root. The skill is named by its frontmatter `name`. Nothing is written if the skill fails the checks. If a skill with that name exists, you are asked whether to replace it.
- **Import skill from a folder** takes a skill folder from your computer. It appears only in browsers that can choose a folder.

The command palette has **New skill** and **Open skill** followed by the skill's name.

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
- **Delete skill**: deletes the skill's folder after you confirm. No agent is offered it afterwards.

## In the apps

The desktop and Android apps list the skills and open the skill screen, but only show them: there is no **+**, no Save and no file changes, and the screen says "Edit skills in the web app". Changing a skill needs `admin`, which an app's device token never has. **Export** and **Download** work in the apps.
