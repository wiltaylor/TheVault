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
