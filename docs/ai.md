# AI providers

TheVault can call large language models with your own credentials. Scripts call them through the `ai` module, and the web app's AI panel uses them to answer questions about the vault. The server makes every call itself: no third party sits between the vault and the provider.

This page covers the three providers, how to connect each in Settings > AI, what the ChatGPT plan allows, what data leaves the server, and the log line every call leaves.

## Providers

| Provider | What you need | Where requests go |
|---|---|---|
| Anthropic | An API key from the [Anthropic Console](https://console.anthropic.com/) | `https://api.anthropic.com/v1/messages`, or the card's API URL |
| OpenAI | An API key from the [OpenAI Platform](https://platform.openai.com/api-keys) | `https://api.openai.com/v1/responses`, or the card's API URL with its API type, Responses (`/responses`) or Chat Completions (`/chat/completions`) |
| ChatGPT | A ChatGPT plan, signed in with OpenAI's Sign in with ChatGPT | `https://api.openai.com/v1/responses`, with the plan's access token |

The two API-key providers bill your API account per token. The ChatGPT provider uses the allowance of your ChatGPT plan instead.

The Anthropic and OpenAI providers can also call another service that speaks the same API, such as a gateway or a model server on your network. See [Use another service](#use-another-service).

## Settings > AI

Settings > AI is in the web app only. The desktop and Android apps pair with a device token, which cannot manage credentials, so the section is absent there; their AI panel uses the home server's settings.

Each provider has a card with:

- Its credential: an API key field and an API URL field, or the ChatGPT sign-in.
- A model list, read live from the provider each time the card opens. No model id is built into TheVault, except that the Anthropic card preselects `claude-opus-5` when Anthropic lists it. The model you pick is that provider's default.
- A **Test** button. It sends a short "Reply with OK" request (16 output tokens at most) to the selected model and shows the result, such as `OK from claude-opus-5 in 840 ms`, or the error kind and message.
- **Use <provider> by default**, which makes it the provider scripts and the AI panel use when they do not name one. The first provider you connect becomes the default.

A script or the AI panel can still name another provider and model for one call.

### Add an API key

1. Open Settings > AI.
2. Paste the key into the Anthropic or OpenAI card's **API key** field and select **Save**.
3. Pick a model from the list, then select **Test**.

The server stores the key encrypted and never sends it back. The field then shows `set` as its placeholder. To replace the key, paste another and save. **Remove key** deletes it.

### Use another service

The Anthropic and OpenAI cards each have an **API URL** field. Leave it empty for the official API; the placeholder shows the official URL. To call another service with that card's API:

1. Type the service's API URL into the **API URL** field.
2. On the OpenAI card, an **API type** select appears beside the URL: **Responses API** or **Chat Completions API**. Leave it on Responses API if you do not know; **Test** finds out (see below).
3. If the service needs a key, paste it into **API key**. With an API URL of its own, the key is optional: a card with a URL and no key counts as connected, and its requests carry no key at all.
4. Select **Save**. It saves the URL and API type first, then the key. **Test** stays unavailable while the card holds an unsaved URL, API type or key, because it uses the saved ones.
5. Pick a model and select **Test**.

How TheVault reads the URL:

- Spaces around it and trailing slashes are dropped.
- A URL whose path ends in a version segment, such as `/v1` or `/v4`, is the API root as it is, and requests go straight under it. Any other URL gets `/v1` added. So `https://openrouter.ai/api/v1`, `https://openrouter.ai/api` and `http://10.0.0.5:11434` all work, and requests go to paths such as `https://openrouter.ai/api/v1/responses`; `https://api.z.ai/api/coding/paas/v4` sends to `https://api.z.ai/api/coding/paas/v4/responses`.
- It must start with `http://` or `https://`. `http` is allowed for servers on your own network; the key, when there is one, then crosses the network unencrypted.
- It must not hold a user name or password (`https://user:pass@...`), a query or a fragment. Put a key in the **API key** field instead.

A URL that breaks these rules is refused with the reason, and the stored URL stays as it was. To go back to the official API, empty the field and select **Save**.

The service must speak the card's API:

- The OpenAI card needs the OpenAI Responses API, `POST /v1/responses`, or the Chat Completions API, `POST /v1/chat/completions`, as its **API type** says. Many services that call themselves OpenAI-compatible offer only Chat Completions. TheVault does not promise that any named service works.
- The Anthropic card needs the Anthropic Messages API, `POST /v1/messages`.

A service that answers 404 or 405 to that path fails with the error kind `invalid` and the message "This service does not support the OpenAI Responses API (POST /v1/responses).", "This service does not support the OpenAI Chat Completions API (POST /v1/chat/completions)." or "This service does not support the Anthropic Messages API (POST /v1/messages)." The paths in these messages are written as under `/v1`; with a versioned root such as `/v4` the request went to `.../v4/chat/completions`.

**Test** detects the OpenAI API type. With a custom URL, when the saved type's path answers 404 or 405, Test sends the same request to the other type's path. When that path answers, Test saves that API type on the card and says so: "OK from glm-5.3-flash in 412 ms (Chat Completions API)". Chats and scripts never probe: they use the saved type, and fail with the message above when the service lacks it. The official OpenAI API (no URL) and the ChatGPT plan always use the Responses API, whatever the select says.

Over Chat Completions, everything works as over Responses, with these differences:

- Tools are sent without `strict`, which many services reject, so the server checks each tool call's input against its schema itself, as it always does.
- A JSON reply (a script's `schema` option) asks for `response_format` `json_schema`. A service that refuses that with a 400 is asked once more with `json_object` and the schema written into the system prompt. The reply is checked against the schema either way.
- A tool message takes only text, so the images `read_asset` returns follow the tool messages in one user message, as `image_url` data URLs.
- A service's reasoning text (`reasoning_content`) is not shown and not kept.
- `finish_reason` `length` means the answer was cut off, and `content_filter` is a refusal with category `content_filter`.

The card lists the service's models from its `GET /v1/models`. When that list fails or is empty, the **Model** field becomes a text box: type the model id the service expects, then press Enter or move to another field.

The ChatGPT plan never uses the OpenAI card's API URL. Its token is for OpenAI alone, so plan requests always go to `https://api.openai.com`.

### Sign in with ChatGPT

TheVault uses OpenAI's official Sign in with ChatGPT flow. It never uses the Codex CLI's client or its stored sign-in.

1. In the ChatGPT card, select **Continue with ChatGPT**.
2. Select **Open the ChatGPT sign-in page**. It opens in a new tab.
3. Sign in to ChatGPT and allow TheVault to use your plan.
4. The browser is then sent to `http://127.0.0.1:53683/auth/callback?...` and fails to load the page. That is expected: nothing listens on that address. Copy the whole address from the address bar.
5. Paste it into **Address from the browser** and select **Finish signing in**. Do this within 10 minutes of step 1.

The card then says **Using ChatGPT plan as** your account's address, with a **Manage usage** link to [ChatGPT's usage settings](https://chatgpt.com/settings/usage).

When you finish, the server checks that the address belongs to the sign-in it started, exchanges the code, and verifies the ID token's signature, issuer, audience, expiry and nonce against OpenAI's published keys. It refuses the sign-in if the plan access (`chatgpt.tokens.use.direct`) was not granted.

The first sign-in registers TheVault with OpenAI, which issues it a client id. Later sign-ins reuse that client id and the same host id, so OpenAI sees the same app on the same server.

**Disconnect** revokes the sign-in at OpenAI and deletes it from the server.

#### Keeping the sign-in

The server refreshes the plan's access token shortly before it expires, so the sign-in keeps working without you. If OpenAI ends the sign-in, for example because you revoked it in ChatGPT, the server forgets the tokens and adds one notification, "ChatGPT sign-in expired", that opens Settings > AI. Select **Continue with ChatGPT** again to reconnect.

## ChatGPT plan limits

Requests through the ChatGPT provider count against your plan's usage, not an API account:

- ChatGPT Pro has no 5-hour limit.
- Each app has a weekly cap, which you can see and change in [ChatGPT's usage settings](https://chatgpt.com/settings/usage).
- When a limit is reached, calls fail with the error kind `rate_limit` until the allowance resets.

The plan also restricts what a request can do. TheVault keeps to these rules: plan requests always stream, are not stored at OpenAI, and carry no output token cap or hosted tools.

## What leaves the server

A call sends the provider the prompt and any text the caller includes:

- A script sends what it passes to `ai::complete` or `ai::chat`, which can include note content.
- The AI panel sends the conversation and the results of the note tools the model calls, such as the notes it searches and reads, and the attachments it reads with `read_asset`: an image itself, a PDF's or text file's text.
- The Test button sends only "Reply with OK."
- With a Brave Search key stored, a `web_search` call sends Brave the query the model wrote; see [Web search](#web-search).
- A `fetch_url` or `download_asset` call requests the address the model chose from the web, as TheVault, and `fetch_url` sends the provider what it read: a page's text, a PDF's text or an image. See [Read pages and download files](#read-pages-and-download-files).

Nothing else from the vault is sent. Responses API requests set `store: false`, so OpenAI keeps no stored response for later reuse. Chat Completions requests, which go only to a custom API URL, carry no such field; the service's own policy decides what it keeps. Each provider's own data policy applies to what it receives.

The keys and tokens are stored encrypted in the server's secrets store, under the names `ai_anthropic_api_key`, `ai_openai_api_key`, `ai_chatgpt` and `ai_settings`. No API response carries a key or token, and error messages are scrubbed of them. A script cannot read them either: the `ai` module reads them itself.

## The trail line

Every completed or failed provider call writes one line:

```text
ai: provider=anthropic model=claude-opus-5 in=1234 out=56 outcome=ok ms=840
```

| Field | Meaning |
|---|---|
| `provider` | `anthropic`, `openai` or `chatgpt` |
| `model` | The model id |
| `in`, `out` | Input and output tokens |
| `outcome` | `ok`, `refused` when the provider refused the request, or `error:<kind>` |
| `ms` | How long the call took |
| `host` | Only for a call to a custom API URL: the service's host, with its port when the URL names one, such as `host=openrouter.ai` |

The error kinds are `not_configured`, `auth`, `rate_limit`, `refusal`, `invalid`, `timeout`, `unavailable` and `provider`.

A script's calls write the line to the run's log. The server's own calls, such as Test, write it to the server log. The prompt text never appears in it.

## Chat panel

The AI panel in the right sidebar answers questions about your notes and proposes edits to them. Open it with the AI tab, Ctrl+Shift+L (Cmd+Shift+L on a Mac), or Ask AI in the command palette (Ctrl+K). The **Agent** menu at the top picks whose conversation it is: the built-in Assistant first, chosen by default, then every enabled agent. The Assistant uses the default provider and model from Settings > AI, and the two menus at the foot of the message box, beside **Send**, pick another for this conversation; an agent that names its own model shows it in their place.

Every conversation in the panel is a thread of the chosen agent (agents G6, G7): see [Threads](#threads).

The panel shows in the web app and in the desktop and Android apps. The chat always runs on the home server, which holds the credentials: an app's embedded server sends the panel's requests there with the device token, so an app needs to be paired and to reach its server. Offline or unpaired, the panel says "The AI chat needs the home server, which is not reachable." When no provider is set up, the app's panel says so and links to Settings > AI on the home server ("Set up AI in the web app"), since Settings > AI is in the web app only.

### Ask a question

Type a question and press Enter (Shift+Enter adds a line; Enter that finishes composing text in an input method only finishes it). The answer streams in as the model writes it. While the turn runs, a line just above the message box shows a spinner, what the model is doing now and the seconds since you asked, such as "Thinking… 4s", "Searching notes… 7s", "Reading a web page… 12s" or "Writing… 15s". It goes when the answer ends, fails or is stopped. With reduced motion the spinner is a still dot, and a screen reader hears each new step but not the seconds. When a note is open beside the panel, the model knows its id, type and title, so "this note" means that note.

To answer, the model calls note tools on the server. Each call shows as a step above the answer, such as “Searched for “roof””, with links to the notes it found or read. The answer links the notes it relies on; a click opens one in the active pane.

| Tool | Does |
|---|---|
| `search_notes` | Full-text search, at most 20 notes |
| `query_notes` | A query in the vault query language, at most 50 notes |
| `get_note` | Reads one note, body included, and lists the assets it references (id, file name, media type, size); each text over 40,000 characters is cut with a marker |
| `read_asset` | Reads one attachment by asset id; see [Attachments](#attachments) |
| `find_assets` | Finds stored files by name: those whose file name contains `text`, ignoring case, newest first, 1 to 50 (`limit`, 20 when absent); empty text lists the newest. Each is its `id`, `filename`, `media_type`, `size` and `created` time |
| `backlinks` | The notes that link to a note |
| `propose_edit` | Checks an edit to one note and shows it to you; writes it only when [confirmation is off](#changes-the-ai-makes) |
| `propose_create` | Checks a new note of a type and shows it to you; creates it only when confirmation is off |
| `propose_archive` | Checks that a note exists and is not already archived (or, to unarchive, archived) and shows it to you; does it only when confirmation is off |
| `propose_delete` | Checks that a note exists and can be deleted and shows it to you; deletes it only when confirmation is off. Applied, the note moves to the trash; the model cannot empty the trash |
| `list_types` | Every type's name, label, plural, description, and whether it is built in |
| `get_type` | Reads one type file's WCL, with its fields and templates |
| `type_format` | The type-file sections of the vault format, so the model knows the vocabulary |
| `propose_type_change` | Checks a whole new type file, for an existing type or a new name, and shows it to you; writes it only when confirmation is off |
| `propose_type_delete` | Checks that a type can be deleted, as Delete type in the type editor would, and shows the delete to you; deletes it only when confirmation is off. A built-in type, a type that still has notes and a type another type's reference field targets are refused, and the error says which |
| `list_taxonomies` | Every taxonomy's name, label, description, whether it is open (notes may add terms), whether it is built in, and its term count |
| `get_taxonomy` | Reads one taxonomy file's WCL, with every term as a path such as `Doing/Blocked` |
| `taxonomy_format` | The taxonomy-file sections of the vault format, and the `taxonomy` field kind |
| `propose_taxonomy_change` | Checks a whole new taxonomy file, for an existing taxonomy or a new name, with its term `renames`, and shows it to you; writes it only when confirmation is off. See [Apply a taxonomy change](#apply-a-taxonomy-change) |
| `propose_taxonomy_delete` | Checks that a taxonomy exists and nothing uses it, and shows the delete to you; deletes it only when confirmation is off |
| `list_agents` | Every agent's name, label, description, provider, model, whether it is enabled and built in, and its trigger count |
| `get_agent` | Reads one agent file's WCL, with the agent parsed and the agent-file vocabulary |
| `propose_agent_change` | Checks a whole new agent file, for an existing agent or a new name, and shows it to you; writes it only when confirmation is off. Refused when it would give the agent more than the proposing agent has. See [Agents that manage agents](ai-agents.md#agents-that-manage-agents) |
| `propose_agent_delete` | Checks that an agent exists and is not the Assistant or the proposing agent, and shows the delete to you; deletes it only when confirmation is off |
| `list_skills`, `get_skill` | Every skill with its metadata, validity and issues, or one skill's `SKILL.md`, body and files. See [Agents that manage skills](ai-agents.md#agents-that-manage-skills) |
| `propose_skill_change`, `propose_skill_file_change`, `propose_skill_file_delete`, `propose_skill_delete` | Check a new or changed `SKILL.md`, a skill file written as text or base64, a file or folder deleted, or a skill deleted, and show it to you; write it only when confirmation is off |
| `web_search` | Searches the web with Brave Search, 1 to 10 results; offered only with a Brave Search key. See [Web search](#web-search) |
| `fetch_url` | Reads a web page or file for the model; never a change. See [Read pages and download files](#read-pages-and-download-files) |
| `download_asset` | Downloads a file of any size into the vault as an asset; stores it only when [confirmation is off](#changes-the-ai-makes) |

A tool result over 100,000 characters is cut with a marker before it goes to the model. One question makes at most 12 calls to the model; after that the panel says it stopped, and asking again goes on.

Stop ends the answer at once: the server stops calling the model when the panel goes away. New conversation forgets the conversation. Conversations live in the browser tab, or the app's window, only and are not saved in the vault.

### Web search

With a Brave Search API key stored in Settings > Integrations > Brave Search ([Web search with Brave Search](brave.md)), the model is offered `web_search { query, count }`. Without a key the tool is not in the model's tool list at all.

- `count` is 1 to 10; a missing one means 5. A query over 400 characters is refused.
- The model gets `[{ title, url, description, age? }]`, with Brave's HTML tags, such as `<strong>`, stripped from the titles and descriptions.
- A Brave error is an error result for the model, with Brave's message and a `kind`: `rate_limit` for 429, `auth` for 401, 403 and 422.
- The panel shows the step as "Searched the web for “<query>”", with the result titles linking out in a new tab.
- Each search writes its own `brave:` line to the server log, which never holds the query; the turn's `ai:` trail line is unchanged.

### Read pages and download files

`fetch_url { url }` reads a page or file for the model. Reading changes nothing, so it is never a proposal. The step shows "Read <host>", the host linking to the address in a new tab.

- The server reads at most 5 MiB. The result starts with the final URL, after redirects, and the media type, then says the content is the page's and not the user's.
- HTML: the page's `<title>`, then its readable text as Markdown-like plain text: headings as `#` lines, paragraphs, lists as `- ` or `1. ` lines, links as `[text](url)` made absolute. Scripts, styles, navigation, page headers and footers, asides, forms and hidden elements are dropped.
- `text/*`, JSON and XML: the text. A PDF: its text, through the extractor the index uses for PDF assets.
- Text of every kind is cut at 40,000 characters with a marker.
- An image: the image itself, sent to the provider and scaled as [Attachments](#attachments) says, counting toward the 8 images per question. It is held for this question only; a later question gets a text saying it is gone, and the model can fetch it again.
- Anything else: the final URL, media type and size.

`download_asset { url, filename }` downloads a file into the vault as an asset. There is no size limit: the server writes the file to `.vault/incoming` as it arrives, never holding it in memory, and moves it into `assets/` once it is whole. A download that fails part way, such as a dropped connection, stores nothing and leaves no file behind. The file name is `filename`, else the last part of the URL's path, else `download`; slashes, backslashes and control characters become `_`, a name without an extension takes one from the media type, and a name over 255 bytes is shortened, keeping its extension.

- With **Ask before downloading files into the vault** on, the default, the server asks the address for its size and media type (a HEAD request, else a GET of the first byte) and the panel shows a card with the address, file name, type and size, and Apply and Discard. Apply sends it to `POST /api/ai/apply`, which downloads the file and stores it.
- With the switch off, the file is stored at once and the panel shows "Downloaded <file name>", with a thumbnail for an image. A caller without `write` always gets the card.
- The model is told the new asset's id, so a later `propose_edit` or `propose_create` can attach it to a note: at once when the file was stored without asking, and with your next message when you applied the card (see [What the model learns you applied](#what-the-model-learns-you-applied)). `find_assets` finds it by name too.

Both tools fetch through one guarded client:

- Only `http` and `https` addresses with a host, and no user name or password in the address.
- **The local-network rule.** The server resolves the host once and refuses the fetch when any address it resolves to is loopback, private (10/8, 172.16/12, 192.168/16), link-local (169.254/16, fe80::/10), unique-local (fc00::/7), carrier-grade NAT (100.64/10), multicast, unspecified, broadcast or 0/8, or the IPv4-mapped IPv6 form of one of these. It connects to the addresses it checked, with no second lookup, and through no proxy. An address written as an IP in the URL is checked the same way. The model gets "TheVault does not fetch addresses on the local network."
- Redirects are followed by the server itself, at most 5, and each target is checked as the first address was. A sixth is refused.
- `fetch_url` has 30 seconds for the whole fetch, redirects and body included. A download has 30 seconds until the answer's headers arrive, then as long as it takes, provided some of the file arrives at least every 60 seconds; a download that stalls longer fails and stores nothing. Requests say `User-Agent: TheVault/<version> (+https://github.com/wiltaylor/TheVault)`.

To let the tools reach servers on your own network, set `"fetch_private": true` in the `ai_settings` secret (`PUT /api/secrets/ai_settings` with the whole settings object). There is no switch for it in Settings > AI: it is an escape hatch, and it lets a page's text steer the model toward services on your network, so leave it off unless you need it.

### The browser box

An agent with **Browser automation** on its Permissions tab, on a server with the [browser box](install.md#browser-box), gets `browser_run`, `browser_file` and `browser_save`: it runs commands in the box, drives its browser with playwright-cli while you watch, reads screenshots and snapshots, and stores files as assets. The switch is off for every agent, the Assistant included. See [Driving the browser box](ai-agents.md#driving-the-browser-box) for the tools, the `browser:` log line and the risks.

### Attachments

The model reads a note's attached files with `read_asset`, by the asset ids `get_note` lists. What it gets depends on the file's media type, which comes from its file name:

| Media type | The model gets |
|---|---|
| `image/png`, `image/jpeg`, `image/gif`, `image/webp` | The image itself, sent to the provider. An image over 3.75 MiB or with a long edge over 2,000 pixels is sent as a JPEG (quality 85) scaled to 1,568 pixels on its long edge |
| Another image type, such as AVIF or SVG | The image converted the same way when the server can decode it; the server decodes only the four types above, so these get the metadata |
| `application/pdf` | The text the index extracted from it, cut at 40,000 characters with a marker. A PDF kept as unreadable, or with no text layer, gets the metadata and a note saying why |
| `text/*` and JSON, which includes Markdown and CSV | The file's text, cut at 40,000 characters with a marker |
| Anything else | The metadata: id, file name, media type and size |

Images are sent to the provider, so whatever a picture shows leaves the server. One question sends at most 8 images; a further image read is refused, and the model is told the limit is reached. The step in the panel shows the image's thumbnail, loaded from the vault like any asset; in an app it loads from the device's copy, and an asset not on the device shows its file name only.

The conversation the browser holds names an image only by its asset id. The server loads the image from the vault, scaled the same way, every time it sends the conversation to the provider, so image bytes never go to the browser or back. An image deleted since the model read it is sent as the text "asset <id> no longer exists".

### Apply an edit

When you ask for a change, such as "add eggs to my shopping list", the model proposes it. The panel shows a card with the changed fields and the lines of text removed and added:

- Apply writes the change as you, on the home server, at the version the model read. If the note changed since, the card says "This note changed since the proposal" and nothing is written; ask again for a fresh proposal.
- Discard drops it. Nothing is written.

The model writes a note itself only when you turn confirmation off; see [Changes the AI makes](#changes-the-ai-makes).

Every client applies a proposal the same way: the panel sends it, as the chat sent it, to `POST /api/ai/apply`, and the home server writes it. In the desktop and Android apps the change is never written to the device's copy directly: after Apply, or after a change the AI made itself, the app asks for a sync round, and the open note or type shows the change once the round brings it.

### Create, archive and delete notes

The model also proposes new notes, archiving a note or taking it out of the archive, and deleting a note. Each card's heading names the operation: "Proposed new note", "Proposed archive of", "Proposed unarchive of" or "Proposed delete of", then the note's title.

- A new note is checked as the create route would check it; a note that does not check goes back to the model with the issues. Its card shows the fields and text it sets. Apply creates it and the card then links to the new note.
- Apply on an archive or unarchive archives the note, or takes it out of the archive, at the version the model saw. If the note changed since, the card says so and writes nothing.
- Apply on a delete moves the note to the trash at the version the model saw, and the card links to the trash, where you can restore it. If the note changed since, the card says so and deletes nothing. Nothing in the AI panel empties the trash.

When an answer leaves two or more proposals waiting, of any kind, the panel shows Apply all under them. It applies them in the order they came and stops at the first that does not apply; that card says why, and the rest wait for you.

### What the model learns you applied

When you select Apply, Apply all or Discard on a card, the panel notes the outcome. Your next message starts with one line for each card acted on since the message before, ahead of your own words, for example:

```text
[Applied by the user: download of https://example.com/cover.jpg stored as asset 01J... (cover.jpg)]
[Applied by the user: created note 01J... "Hades II", version v1]
[Applied by the user: edit of note 01J... now at version v3]
[Discarded by the user: download of https://example.com/map.png]
[Failed to apply: edit of note 01J...: This note changed since the proposal, so it was not applied. Ask again for a fresh proposal.]
```

- The lines go as a separate text block at the start of the user message, so every provider sees them, and the conversation stays append-only.
- The panel shows them above your message, collapsed, as **Applied changes**, not as your words.
- Each card counts once: a card that failed and then applied sends only the applied line, and the lines are not sent again with the message after.
- Other outcome words: `archive of note <id> now at version <v>`, `unarchive of note <id> ...`, `delete of note <id>: moved to the trash`, `change to type <name>`, `created type <name>`, `changed taxonomy <name>` (with `, its renames moving <n> notes` when they moved any), `created taxonomy <name>`, `deleted taxonomy <name>`, and for a discard or failure `change to taxonomy <name>`, `new taxonomy <name>` and `delete of taxonomy <name>`.

### Apply a type change

When you ask to change a type, such as "add a due date to tasks" or "put the status on its own tab", the model reads the type and proposes a whole new type file. The server checks it as the type write would; WCL that does not check goes back to the model with the issues, and the model fixes it before you see anything. A new name proposes a new type. The card shows the type file's lines removed and added:

- Apply sends it to the home server, which checks the type file is still the one the proposal was made against. If it changed since, the card says "This type changed since the proposal" and nothing is written; ask again for a fresh proposal. Otherwise the server writes it as the type routes do: a new type as `PUT /api/types/{name}` does, a change as `POST /api/types/{name}/change` does, planned and then applied as one commit. Notes of the type are changed with it as that plan says.
- Discard drops it. Nothing is written.

Applying a type change needs `admin`, as the type editor does. A device token has `write` only, so in the desktop and Android apps the card answers "Apply type changes in the web app." Built-in types allow what the type routes allow for them.

### Apply a taxonomy change

When you ask to change a taxonomy, such as "rename Todo to Open in the status taxonomy" or "add a size taxonomy", the model reads it and proposes a whole new taxonomy file with a list of renames, each an old term path and its new one. A renamed or moved term takes its children with it. The server checks the proposal as `PUT /api/taxonomies/{name}` would, without writing: WCL that does not check, a rename of a term the taxonomy lacks, and a file that drops a term some note holds without renaming it all go back to the model with the issues and the notes concerned. A new name proposes a new taxonomy. The card shows the file's lines removed and added, the renames, and how many notes they move:

- Apply sends it to the home server, which checks the taxonomy file is still the one the proposal was made against. If it changed since, the card says "This taxonomy changed since the proposal" and nothing is written. Otherwise the server writes it as `PUT /api/taxonomies/{name}` does, with the renames, rewriting every note that holds a renamed term in the same commit.
- Discard drops it. Nothing is written.

The model can also propose deleting a taxonomy. A taxonomy a type's field uses cannot be deleted: the model is told which types and notes use it, and no card appears. Apply on a delete card deletes the file as `DELETE /api/taxonomies/{name}` does. The cards link to Settings > Taxonomies.

Applying a taxonomy change needs `admin`, as the taxonomy editor does; in the desktop and Android apps the card answers "Apply taxonomy changes in the web app."

### Threads

Each conversation with an agent is a note of the built-in type `agent_thread` (agents G7). The server writes it as the turn streams, so closing the tab loses nothing, and reads its transcript itself for the next turn, so the client sends only the new question. The thread holds the transcript the providers replay, every tool step, and every proposal with what became of it: pending, applied, discarded or failed (agents G8). The note's `body` is the conversation as plain text, so a search finds it, and its `title` is the first question until you rename it. Every write to a thread is authored by the agent.

The agent screen, at `/agents/<name>`, opens on the agent's chat. The bar at its top holds the session picker, **Rename**, **Archive** and **Delete** for the chosen session, and the settings cog. The session picker lists the agent's threads, newest activity first, with a badge counting the proposals that still wait for you; its search box keeps the threads whose title holds every word typed, the arrow keys move through them, Enter opens the one highlighted and Escape closes the list. **New chat**, at the top of the list, starts another; its first message creates the note. The address names the open thread as `?thread=<id>`. Opening a thread shows it as it was, with Apply and Discard on its pending proposals. Apply and Discard are recorded in the thread, and the next question tells the model what became of each, as [What the model learns you applied](#what-the-model-learns-you-applied) describes. **Rename** edits the chosen thread's title in place, **Archive** archives it or, on an archived thread, **Unarchive** takes it out of the archive, and **Delete** asks first, then moves the note to the trash and starts a new chat. Archived threads are hidden from the list until **Show archived**, at its foot, lists them dimmed; an archived thread stays open after Archive, marked "Archived", and sending a message in it takes it out of the archive. The panel's thread, archived or deleted elsewhere, gives way to a new thread on the next message. Deleting an agent keeps its threads.

The note tools leave threads out unless an agent's `permissions.types` names `agent_thread`, so an agent does not read chats back as vault content. A turn by an agent other than the Assistant authors its changes as that agent, and so does applying its proposal.

An agent can also run on its own, on a schedule or when notes change, each run in a new thread. [AI agents in the vault](ai-agents.md) covers agent files, permissions, triggers, their limits and why an agent's changes never set off a trigger.

### Context and compaction

Every model call sends the thread's whole transcript, so a long thread fills the model's context window. The meter above the chat box shows how full it is after the last call: the tokens the call took in and gave back over the window, such as `42k / 200k`. It turns amber past the compaction point and red near the window.

Compacting asks the model, with the agent's own instructions and no tools, for a summary of the thread so far, and from then on sends the summary in place of the messages it covers, at the head of the next message. The thread keeps every message for you to read; a divider marks where the summary begins and opens it. A later compaction summarizes the earlier summary and what came after it.

- **On its own.** Before a turn, a thread whose last call filled the compaction point of the window, 80% unless Settings > AI says otherwise, is compacted first, and the turn then answers from the summary. If the summary fails, the turn goes on with the thread as it was and the panel says so.
- **`/compact`.** Typed alone in the chat box, it compacts the thread now and ends the turn there. It works in a thread; the panel always has one.

Settings > AI holds the context window of each provider, from 4,000 to 10,000,000 tokens, and when to compact, from 50% to 95%. A window left at its default is 200,000 for Anthropic, 400,000 for the OpenAI API (128,000 for a service the OpenAI card reaches over Chat Completions) and 272,000 for the ChatGPT plan. **Compact long chats on their own** turns the automatic compaction off; `/compact` still works. The windows are the sizes TheVault plans with, not limits it enforces: a model with a smaller window than the one set answers a full thread with an error.

`GET /api/integrations/ai` and `GET /api/ai/providers` answer `context: { windows: { <provider>: { window, set } }, auto_compact, compact_at }`; `PUT /api/integrations/ai/settings` takes `context: { windows?: { <provider>: <tokens> | null }, auto_compact?, compact_at? }`, `null` putting a window back to its default. A thread's JSON carries `context: { used, window }` and `compaction: { summary, upto, at_ms, before, reason }`.

### The system prompt

The system prompt is the Assistant's (agents G6): edit it in the Assistant's agent screen, under Settings, where **Use the shipped prompt** goes back to the prompt TheVault ships. Every other agent has its own. The prompt can be at most 32,000 characters. A vault from before agents moves the prompt it had in Settings > AI to the Assistant on its first start.

The server adds the agent's memories (see [Memory](ai-agents.md#memory)) and then the open note's id, type and title after your prompt, so the editable text never holds them. The tool list reaches the model through the tool definitions, so a custom prompt does not need to repeat it. The default prompt also summarises the query language and how to link notes; keep those parts if you want the model to go on doing both well.

Scripts do not use this prompt: a script's `ai` call passes its own `system`.

### Changes the AI makes

By default every change the model asks for waits for you as a proposal. Each agent's Permissions tab has seven confirmation switches, all on by default; the panel's are the Assistant's (agents G4, G6), and a vault from before agents moves the ones it had in Settings > AI there on its first start:

| Switch | Covers |
|---|---|
| Ask before creating and editing notes | `propose_edit` and `propose_create` |
| Ask before archiving notes | `propose_archive`, archiving and unarchiving |
| Ask before moving notes to the trash | `propose_delete` |
| Ask before changing types | `propose_type_change` and `propose_type_delete`, changed, new and deleted types |
| Ask before downloading files into the vault | `download_asset`; see [Read pages and download files](#read-pages-and-download-files) |
| Ask before changing taxonomies | `propose_taxonomy_change` and `propose_taxonomy_delete`, changed, new and deleted taxonomies |
| Ask before changing agents | `propose_agent_change` and `propose_agent_delete`, changed, new and deleted agents (see [Agents that manage agents](ai-agents.md#agents-that-manage-agents)), and `propose_agent_memory_change` and `propose_agent_memory_delete`, other agents' memories (see [Other agents' memories](ai-agents.md#other-agents-memories)) |

Each switch saves when you change it. With a switch off, the tool makes that kind of change itself, right after checking it, through the same writes Apply uses:

- An edit is written at the version the model read. If the note changed since, nothing is written and the model is told of the conflict, so it reads the note again.
- A delete moves the note to the trash, where you can restore it. The AI never empties the trash or deletes a note for good, whatever the switches say.
- A type change is planned and applied as one commit, as `POST /api/types/{name}/change` does; a new type is written as `PUT /api/types/{name}` does.
- A taxonomy change is written with its renames as `PUT /api/taxonomies/{name}` does, and a delete as `DELETE /api/taxonomies/{name}` does.
- An agent change is written as `PUT /api/agents/{name}` does, and a delete as `DELETE /api/agents/{name}` does, after checking it again against the proposing agent.

### Internet switches

The Permissions tab also has an **Internet** group, which decides which web tools the agent is given at all. All three are on by default.

| Switch | Tool |
|---|---|
| Let the AI search the web | `web_search`, which also needs a Brave Search key in Settings > Integrations |
| Let the AI read web pages | `fetch_url` |
| Let the AI download files into the vault | `download_asset`, which still follows Ask before downloading files into the vault |

With a switch off, the tool is left out of the tool list, so the model never sees it, and a call naming it anyway is refused without fetching anything. `GET /api/integrations/ai` and `GET /api/ai/providers` answer the Assistant's `confirm` and `web`, and `PUT /api/integrations/ai/settings` still takes a partial `confirm`, `web` and `system_prompt`, which it writes into the Assistant's file.

A change that does not check, or that fails, writes nothing and goes back to the model as an error. The model is told the change was made, with the note's new version.

The panel shows each change made this way as a short card, such as "Edited Shopping", "Created Reading list", "Archived Old plan", "Moved Scratch to the trash", "Changed type task", "Changed taxonomy status" or "Changed agent clerk", linking to the note, the trash, the type, Settings > Taxonomies or the agent's screen. A note or type open in a pane shows the change at once, unless you have unsaved edits in it.

The change is written as the agent when the request names one, else as whoever asked: the signed-in user in the web app, or the token's name for an API caller. It is in the note's history under that name like any other write; the trail line is the same as for any chat call. The switches never widen a caller's rights: a caller whose scope cannot make the write still gets a proposal. Notes need `write`, types, taxonomies, agents and skills need `admin`, so a `read` token always gets proposals, and a `write` token gets proposals for type, taxonomy, agent and skill changes.

### Refusals and errors

When the provider refuses a request, the answer stops where it was and the panel shows which model refused and the category the provider gave, such as `cyber`. TheVault never retries a refusal with another model. Other failures show the error message, for example a rate limit.

### The chat route

`POST /api/ai/chat` needs a `read` token or the web app's session. Applying a proposal with `POST /api/ai/apply` needs `write`, and `admin` for a type or taxonomy proposal. A change the chat makes itself, its confirmation off, needs the same scope from the chat's caller; without it the chat proposes instead. The body is `{ provider?, model?, messages, note_id?, agent?, thread?, update_skills? }`, where `messages` is the conversation so far ending with the new question. `agent` names the agent whose turn it is: its model (when it names one), prompt, effort, tools, note types and switches apply, and a disabled agent is a 400 `agent_disabled`; without it the turn is the Assistant's. `thread` names one of the agent's threads: the server reads the transcript from the thread, so `messages` holds only the new question, and writes the turn to it; a thread of another agent is a 400, and a thread a turn is already running on a 409 `thread_busy`. `update_skills: true` is `/update-skills` (skills S6): the thread's snapshot of the agent's skills is taken again, `messages` holds none of the user's (an empty list on a thread), and the turn's user message is the server's, sent first as the `skills_updated` event (see [Using skills](ai-agents.md#using-skills)). With an agent named, each `proposal`, `type_proposal`, `taxonomy_proposal` and `applied` event also carries `agent`, and with a thread `thread`. The answer is a stream of server-sent events, with a heartbeat comment every 15 seconds:

| Event | Data |
|---|---|
| `text` | `{ delta }`, the next piece of the answer |
| `tool` | `{ id, name, input }`, a tool call about to run |
| `tool_result` | `{ id, summary, is_error }`; `summary.notes` lists the ids and titles returned, never note bodies; for `read_asset`, `summary.asset` is `{ id, filename, media_type, size, kind }`, never the file's bytes or text; for `find_assets`, `summary.assets` is the `[{ id, filename, media_type, size, created }]` it returned and `summary.total` how many matched; for `fetch_url` and `download_asset`, `summary.web` is `{ url, host, media_type, size, kind }`, with a page's `title` or a download's `filename` (and `asset_id` once stored), never the content |
| `proposal` | A note change, by `op`, to apply with `POST /api/ai/apply`. `update`: `{ id, op, note_id, title, base_version, before, after, ops }`, an edit. `create`: `{ id, op, type, title, after }`, a new note. `archive` and `unarchive`: `{ id, op, note_id, title, base_version }`. `delete`: `{ id, op, note_id, title, base_version }`, a move to the trash. `download`: `{ id, op, url, filename, media_type, size }`, a file to store as an asset (`media_type` and `size` may be null). A proposal without `op` is an `update` |
| `type_proposal` | `{ id, name, exists, before, after, issues }`: `before` is the type file as stored (null for a new type), `after` the checked type file in canonical form, `issues` the warnings the check found. A delete is `{ id, kind, op, name, exists, before, after, issues }` with `kind` `type`, `op` `delete` and `after` null |
| `taxonomy_proposal` | A taxonomy change to apply with `POST /api/ai/apply`, `kind` always `taxonomy`. `create` and `change`: `{ id, kind, op, name, exists, before, after, renames, issues, affected }`, where `before` is the file as stored (null for a new taxonomy), `after` the checked file in canonical form, `renames` the `[{ from, to }]` term paths and `affected` how many notes they rewrite. `delete`: `{ id, kind, op, name, before }` |
| `applied` | A change the tool made itself, its confirmation off, sent in place of the proposal. For a note, `{ id, op, note_id, title, version }`, where `op` is `update`, `create`, `archive`, `unarchive` or `delete` and `version` is the note's new version (for a delete, the version that went to the trash). For a type, `{ id, op, type_name }`, where `op` is `type_change`, `type_create` or `type_delete`. For a taxonomy, `{ id, op, taxonomy_name, notes }`, where `op` is `taxonomy_change`, `taxonomy_create` or `taxonomy_delete` and `notes` the number of notes its renames rewrote. For a download, `{ id, op, asset_id, filename, media_type, size, url }` |
| `skills_updated` | `{ count, skills, message }`, first in an `update_skills` turn: how many skills the new snapshot lists, their names, and the user message the server added, a `skills_updated` notice ("Skills updated (<n>)") and the text the model reads. A conversation with no thread appends `message` to the transcript it sends back |
| `usage` | `{ provider, model, input, output }` for one model call |
| `refusal` | `{ provider, model, category, explanation, messages }`; the last event |
| `error` | `{ kind, message, messages }`; the last event |
| `done` | `{ stop, messages }`; the last event |

`messages` holds the turn's new conversation items, including the provider's own record of each answer. Send them back unchanged, after the earlier ones, with the next question. Each call to the model writes a trail line to the server log.

### The turns form

Every client runs its turns in two steps: the web app, and the desktop and Android apps, whose webviews cannot read a streamed answer to a POST. Both forms run the turn the same way and send the same events, but a turn in this form runs on the server whether or not anyone reads it (agents G17): switching threads, closing the panel or the tab never stops it.

| Method and path | Does |
|---|---|
| `POST /api/ai/chat/turns` | Takes the body `POST /api/ai/chat` takes, starts the turn, and answers 201 `{ turn, started_ms }`. At most 4 turns run at once per caller, and per agent, counting the agent's trigger runs; one more is a 429 `too_many_turns`. A turn on a thread that already has one running is a 409 `thread_busy` |
| `GET /api/ai/chat/turns/{turn}/events?after=<n>` | Streams the turn's events after the first `n` (all of them without `after`): those sent so far, then the rest as they come, as `POST /api/ai/chat` would, with the heartbeat. Each event's SSE `id` is its number from 1, and `EventSource`'s `Last-Event-ID` counts as `after`. Any number of readers may read one turn |
| `DELETE /api/ai/chat/turns/{turn}` | Stops the turn, 204. Stop is the only way to end a turn on a thread early |

All three need `read`, and a turn is its caller's: another caller's turn is a 404 `unknown_turn`, as is a stopped turn. A finished turn's events are kept for 10 minutes; after that they answer 410 `turn_expired`, and the client reads the thread, which holds the whole turn. A turn with no `thread` stops when nobody has read it for 30 seconds. Turns are held in the server's memory.

A turn on a thread starting and ending is announced on `/api/events` as `ai_turn` `{ agent, thread, turn, running }` (`turn` is null for a trigger's run), and `GET /api/agents/{name}/threads` answers `running`, `turn` and `started_ms` for each thread, so the agent screen's session picker and the sidebar show a spinner on a running thread and its agent, and a thread that answered while it was not open shows "New reply" until opened. Returning to a running thread reads its turn again from the start.

In the desktop and Android apps the embedded server forwards these routes to the home server with the device token. The webview cannot read a stream through the app's custom scheme either, so it polls the embedded server's `GET /api/app/ai/turns/{turn}?after=<n>`, which reads the turn's stream from the home server and answers `{ events: [{ event, data }], next, done }`: the events after the first `n`, waiting up to 2 seconds for one. The panel asks again with `after` set to `next` until `done`. A turn the panel stops asking about for 30 seconds is let go here; it runs on on the home server, and a later poll reads it again from there.

### Applying a change

`POST /api/ai/apply` takes a `proposal`, `type_proposal` or `taxonomy_proposal` event's data, as the chat sent it, and applies it on the server as the caller, through the same code the chat uses when [confirmation is off](#changes-the-ai-makes). A taxonomy proposal is told apart by its `kind`, `taxonomy`, and a type proposal by its WCL `after` text.

- A note change is written at its `base_version`: an update, an archive or unarchive, or a delete, which moves the note to the trash. A create makes the note. It needs `write`. A note that changed since is a 409 `conflict`, and nothing is written.
- An agent's type, taxonomy, agent or skill proposal that waits in its thread may be applied by a caller with `write`, such as an app's device token: the server applies the proposal as the thread holds it, not the body sent, so the user applies these from the AI tab of the apps too. Any other such change needs `admin`, as below.
- A type change or delete needs `admin`; without it the answer is 403 `needs_admin`, "Apply type changes in the web app." A type file that changed since the proposal is a 409 `stale`. A delete is refused as `DELETE /api/types/{name}` refuses it: a type that has notes by then is a 409 `type_in_use`, a built-in type a 409 `built_in_type`.
- A taxonomy change or delete needs `admin`; without it the answer is 403 `needs_admin`, "Apply taxonomy changes in the web app." A taxonomy file that changed since the proposal is a 409 `stale`. The write refuses as the taxonomy routes do: a dropped term notes hold is a 409 `term_in_use`, a taxonomy in use a 409 `taxonomy_in_use`, and nothing is written.
- A change to another agent's memories (its `kind` is `agent_memory`, see [Other agents' memories](ai-agents.md#other-agents-memories)) needs `write`. It is checked again against the agent that proposed it, a 403 `not_permitted` when it may no longer reach those memories, and a memory that changed since the proposal is a 409 `stale`. It answers `{ op, memory_id, memory_agent, version }`, where `op` is `agent_memory_create`, `agent_memory_change` or `agent_memory_delete`.
- A download needs `write`. The server downloads `url` through the guarded client and stores it as a new asset called `filename`, answering `{ op: "download", asset_id, filename, media_type, size, url }`. An address the local-network rule refuses is a 403 `local_network`, and a fetch that fails, part way included, a 502 `fetch_failed`; nothing is stored.
- It answers `{ op, note_id, version }` for a note, with the note's new version (for a delete, the version that went to the trash), `{ op, type_name }` for a type, where `op` is `type_change`, `type_create` or `type_delete`, and `{ op, taxonomy_name, notes }` for a taxonomy, where `op` is `taxonomy_change`, `taxonomy_create` or `taxonomy_delete` and `notes` is how many notes its renames rewrote.
- With the proposal's `agent`, the change is written as that agent, and a note of a type outside its `permissions.types` is a 403 `not_permitted`. With its `thread`, the outcome, applied with the answer, or failed or stale with the reason, is recorded in the thread.
- Anything that is not a change the chat proposes is a 400.

`POST /api/ai/discard` (`read`) takes `{ id, thread }` and records that the user discarded the thread's proposal `id`. It changes nothing else.

The thread routes, `/api/agents/{name}/threads` (`GET` lists the agent's threads, newest activity first, as `{ threads: [{ id, title, origin, last_activity, pending, archived }] }`, leaving archived threads out unless `?archived=true`; `POST` with `{ title? }` starts one, both `read`) and `/api/agents/{name}/threads/{id}` (`GET` answers the thread with its `messages`, `steps` and `proposals`, `read`; `PATCH` with `{ title }` renames it and `DELETE` moves it to the trash, both `write`), are forwarded to the home server by the apps' embedded server.

## API

Settings > AI uses these routes. Every route under `/api/integrations/ai` needs an `admin` token or the web app's session; `GET /api/ai/models` and `GET /api/ai/providers` need `read`.

| Method and path | Does |
|---|---|
| `GET /api/integrations/ai` | Each provider's state: `configured`, the default `model`, the ChatGPT `account`, and for Anthropic and OpenAI the API `url` (null for the official API) and `has_key`, and for OpenAI the `api` its calls use, `responses` or `chat_completions` (always `responses` without a URL); the `default_provider`; the chat `system_prompt` in use, the shipped `default_system_prompt`, and `system_prompt_custom`; and `confirm`, `{ notes, archive, delete, types, downloads, taxonomies, agents, skills }`, with every switch filled |
| `PUT /api/integrations/ai/settings` | Saves `{ default_provider?, models?: { anthropic?, openai?, chatgpt? }, system_prompt?, confirm?: { notes?, archive?, delete?, types?, downloads?, taxonomies?, agents?, skills? }, urls?: { anthropic?, openai? }, openai_api? }`; an empty string clears a value, an empty `system_prompt` means the default, an empty URL the official API, and an empty `openai_api` the Responses API. Another `openai_api` than `responses` or `chat_completions` is a 400. A `confirm` sent changes only the switches it names. A `system_prompt` over 32,000 characters is a 400. A URL the rules above refuse is a 400 `invalid_url`, and nothing is saved |
| `PUT /api/integrations/ai/{anthropic\|openai}/key` | Stores `{ key }`; an empty key deletes it |
| `POST /api/integrations/ai/{provider}/test` | Sends the test request to `{ model }` or the default model, and answers `{ ok, model, ms, text, api, error }`. On OpenAI with a custom URL it detects and saves the API type as above, and `api` names the one the answer came over; otherwise `api` is null |
| `POST /api/integrations/ai/chatgpt/start` | Answers `{ auth_url }`, the ChatGPT sign-in page |
| `POST /api/integrations/ai/chatgpt/finish` | Takes `{ url }`, the pasted address, and stores the sign-in; a refused sign-in is a 400 with the reason |
| `POST /api/integrations/ai/chatgpt/disconnect` | Revokes and deletes the ChatGPT sign-in |
| `GET /api/ai/models?provider=` | The provider's models now, `{ provider, model, models: [{ id, display_name }] }`; without `provider`, the default one |
| `GET /api/ai/providers` | The chat panel's picker in every client: `{ default_provider, providers: { anthropic, openai, chatgpt: { configured, model } }, confirm }`. It holds no account, no system prompt, no API URL and never a key or token |
