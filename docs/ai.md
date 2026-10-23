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

The AI panel in the right sidebar answers questions about your notes and proposes edits to them. Open it with the AI tab, Ctrl+Shift+L (Cmd+Shift+L on a Mac), or Ask AI in the command palette (Ctrl+K). It uses the default provider and model from Settings > AI; the two menus at the top of the panel pick another for this conversation.

The panel shows in the web app and in the desktop and Android apps. The chat always runs on the home server, which holds the credentials: an app's embedded server sends the panel's requests there with the device token, so an app needs to be paired and to reach its server. Offline or unpaired, the panel says "The AI chat needs the home server, which is not reachable." When no provider is set up, the app's panel says so and links to Settings > AI on the home server ("Set up AI in the web app"), since Settings > AI is in the web app only.

### Ask a question

Type a question and press Enter (Shift+Enter adds a line). The answer streams in as the model writes it. When a note is open beside the panel, the model knows its id, type and title, so "this note" means that note.

To answer, the model calls note tools on the server. Each call shows as a step above the answer, such as “Searched for “roof””, with links to the notes it found or read. The answer links the notes it relies on; a click opens one in the active pane.

| Tool | Does |
|---|---|
| `search_notes` | Full-text search, at most 20 notes |
| `query_notes` | A query in the vault query language, at most 50 notes |
| `get_note` | Reads one note, body included, and lists the assets it references (id, file name, media type, size); each text over 40,000 characters is cut with a marker |
| `read_asset` | Reads one attachment by asset id; see [Attachments](#attachments) |
| `backlinks` | The notes that link to a note |
| `propose_edit` | Checks an edit to one note and shows it to you; writes it only when [confirmation is off](#changes-the-ai-makes) |
| `propose_create` | Checks a new note of a type and shows it to you; creates it only when confirmation is off |
| `propose_archive` | Checks that a note exists and is not already archived (or, to unarchive, archived) and shows it to you; does it only when confirmation is off |
| `propose_delete` | Checks that a note exists and can be deleted and shows it to you; deletes it only when confirmation is off. Applied, the note moves to the trash; the model cannot empty the trash |
| `list_types` | Every type's name, label, plural, description, and whether it is built in |
| `get_type` | Reads one type file's WCL, with its fields and templates |
| `type_format` | The type-file sections of the vault format, so the model knows the vocabulary |
| `propose_type_change` | Checks a whole new type file, for an existing type or a new name, and shows it to you; writes it only when confirmation is off; never deletes a type |
| `web_search` | Searches the web with Brave Search, 1 to 10 results; offered only with a Brave Search key. See [Web search](#web-search) |

A tool result over 100,000 characters is cut with a marker before it goes to the model. One question makes at most 12 calls to the model; after that the panel says it stopped, and asking again goes on.

Stop ends the answer at once: the server stops calling the model when the panel goes away. New conversation forgets the conversation. Conversations live in the browser tab, or the app's window, only and are not saved in the vault.

### Web search

With a Brave Search API key stored in Settings > Integrations > Brave Search ([Web search with Brave Search](brave.md)), the model is offered `web_search { query, count }`. Without a key the tool is not in the model's tool list at all.

- `count` is 1 to 10; a missing one means 5. A query over 400 characters is refused.
- The model gets `[{ title, url, description, age? }]`, with Brave's HTML tags, such as `<strong>`, stripped from the titles and descriptions.
- A Brave error is an error result for the model, with Brave's message and a `kind`: `rate_limit` for 429, `auth` for 401, 403 and 422.
- The panel shows the step as "Searched the web for “<query>”", with the result titles linking out in a new tab.
- Each search writes its own `brave:` line to the server log, which never holds the query; the turn's `ai:` trail line is unchanged.

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

### Apply a type change

When you ask to change a type, such as "add a due date to tasks" or "put the status on its own tab", the model reads the type and proposes a whole new type file. The server checks it as the type write would; WCL that does not check goes back to the model with the issues, and the model fixes it before you see anything. A new name proposes a new type. The card shows the type file's lines removed and added:

- Apply sends it to the home server, which checks the type file is still the one the proposal was made against. If it changed since, the card says "This type changed since the proposal" and nothing is written; ask again for a fresh proposal. Otherwise the server writes it as the type routes do: a new type as `PUT /api/types/{name}` does, a change as `POST /api/types/{name}/change` does, planned and then applied as one commit. Notes of the type are changed with it as that plan says.
- Discard drops it. Nothing is written.

Applying a type change needs `admin`, as the type editor does. A device token has `write` only, so in the desktop and Android apps the card answers "Apply type changes in the web app." Built-in types allow what the type routes allow for them.

### The system prompt

The **Chat system prompt** card in Settings > AI holds the instructions the panel sends the model before every conversation, for every provider. It shows the prompt in use and whether it is the **Default** or **Customised**:

- Edit the text and select **Save**. The next question uses it. The prompt can be at most 32,000 characters.
- **Reset to default** goes back to the prompt TheVault ships. It is disabled while the default is in use. Saving an empty prompt, or the default text unchanged, also resets it.

The server adds the open note's id, type and title after your prompt, so the editable text never holds them. The tool list reaches the model through the tool definitions, so a custom prompt does not need to repeat it. The default prompt also summarises the query language and how to link notes; keep those parts if you want the model to go on doing both well.

Scripts do not use this prompt: a script's `ai` call passes its own `system`.

### Changes the AI makes

By default every change the model asks for waits for you as a proposal. The **Changes the AI makes** card in Settings > AI has four switches, all on by default:

| Switch | Covers |
|---|---|
| Ask before creating and editing notes | `propose_edit` and `propose_create` |
| Ask before archiving notes | `propose_archive`, archiving and unarchiving |
| Ask before moving notes to the trash | `propose_delete` |
| Ask before changing types | `propose_type_change`, changed and new types |

Each switch saves when you change it. With a switch off, the tool makes that kind of change itself, right after checking it, through the same writes Apply uses:

- An edit is written at the version the model read. If the note changed since, nothing is written and the model is told of the conflict, so it reads the note again.
- A delete moves the note to the trash, where you can restore it. The AI never empties the trash or deletes a note for good, whatever the switches say.
- A type change is planned and applied as one commit, as `POST /api/types/{name}/change` does; a new type is written as `PUT /api/types/{name}` does.

A change that does not check, or that fails, writes nothing and goes back to the model as an error. The model is told the change was made, with the note's new version.

The panel shows each change made this way as a short card, such as "Edited Shopping", "Created Reading list", "Archived Old plan", "Moved Scratch to the trash" or "Changed type task", linking to the note, the trash or the type. A note or type open in a pane shows the change at once, unless you have unsaved edits in it.

The change is written as whoever asked: the signed-in user in the web app, or the token's name for an API caller. It is in the note's history under that name like any other write; the trail line is the same as for any chat call. The switches never widen a caller's rights: a caller whose scope cannot make the write still gets a proposal. Notes need `write`, types need `admin`, so a `read` token always gets proposals, and a `write` token gets proposals for type changes.

### Refusals and errors

When the provider refuses a request, the answer stops where it was and the panel shows which model refused and the category the provider gave, such as `cyber`. TheVault never retries a refusal with another model. Other failures show the error message, for example a rate limit.

### The chat route

`POST /api/ai/chat` needs a `read` token or the web app's session. Applying a proposal with `POST /api/ai/apply` needs `write`, and `admin` for a type proposal. A change the chat makes itself, its confirmation off, needs the same scope from the chat's caller; without it the chat proposes instead. The body is `{ provider?, model?, messages, note_id? }`, where `messages` is the conversation so far ending with the new question. The answer is a stream of server-sent events, with a heartbeat comment every 15 seconds:

| Event | Data |
|---|---|
| `text` | `{ delta }`, the next piece of the answer |
| `tool` | `{ id, name, input }`, a tool call about to run |
| `tool_result` | `{ id, summary, is_error }`; `summary.notes` lists the ids and titles returned, never note bodies; for `read_asset`, `summary.asset` is `{ id, filename, media_type, size, kind }`, never the file's bytes or text |
| `proposal` | A note change, by `op`, to apply with `POST /api/ai/apply`. `update`: `{ id, op, note_id, title, base_version, before, after, ops }`, an edit. `create`: `{ id, op, type, title, after }`, a new note. `archive` and `unarchive`: `{ id, op, note_id, title, base_version }`. `delete`: `{ id, op, note_id, title, base_version }`, a move to the trash. A proposal without `op` is an `update` |
| `type_proposal` | `{ id, name, exists, before, after, issues }`: `before` is the type file as stored (null for a new type), `after` the checked type file in canonical form, `issues` the warnings the check found |
| `applied` | A change the tool made itself, its confirmation off, sent in place of the proposal. For a note, `{ id, op, note_id, title, version }`, where `op` is `update`, `create`, `archive`, `unarchive` or `delete` and `version` is the note's new version (for a delete, the version that went to the trash). For a type, `{ id, op, type_name }`, where `op` is `type_change` or `type_create` |
| `usage` | `{ provider, model, input, output }` for one model call |
| `refusal` | `{ provider, model, category, explanation, messages }`; the last event |
| `error` | `{ kind, message, messages }`; the last event |
| `done` | `{ stop, messages }`; the last event |

`messages` holds the turn's new conversation items, including the provider's own record of each answer. Send them back unchanged, after the earlier ones, with the next question. Each call to the model writes a trail line to the server log.

### The turns form

A client that cannot read a streamed answer to a POST, such as the Android app's webview, which gets whole bodies, runs a turn in two steps instead. Both forms run the turn the same way and send the same events.

| Method and path | Does |
|---|---|
| `POST /api/ai/chat/turns` | Takes the body `POST /api/ai/chat` takes, starts the turn, and answers 201 `{ turn }`. At most 4 turns run at once per caller; one more is a 429 `too_many_turns` |
| `GET /api/ai/chat/turns/{turn}/events` | Streams the turn's events, as `POST /api/ai/chat` would, with the heartbeat; usable with `EventSource`. A turn has one reader: a second one is a 409 `turn_read` |
| `DELETE /api/ai/chat/turns/{turn}` | Stops the turn, 204 |

All three need `read`, and a turn is its caller's: another caller's turn is a 404 `unknown_turn`, as is a turn that ended and was read, was stopped, or was never read. A turn nobody reads within 30 seconds stops, and so does a turn whose reader goes away, as a `POST /api/ai/chat` whose client goes away does. Turns are held in the server's memory.

In the desktop and Android apps the embedded server forwards these routes to the home server with the device token. The webview cannot read a stream through the app's custom scheme either, so it polls the embedded server's `GET /api/app/ai/turns/{turn}?after=<n>`, which reads the turn's stream from the home server and answers `{ events: [{ event, data }], next, done }`: the events after the first `n`, waiting up to 2 seconds for one. The panel asks again with `after` set to `next` until `done`. A turn the panel stops asking about for 30 seconds is let go, which stops it on the home server.

### Applying a change

`POST /api/ai/apply` takes a `proposal` or `type_proposal` event's data, as the chat sent it, and applies it on the server as the caller, through the same code the chat uses when [confirmation is off](#changes-the-ai-makes). A type proposal is told apart by its WCL `after` text.

- A note change is written at its `base_version`: an update, an archive or unarchive, or a delete, which moves the note to the trash. A create makes the note. It needs `write`. A note that changed since is a 409 `conflict`, and nothing is written.
- A type change needs `admin`; without it the answer is 403 `needs_admin`, "Apply type changes in the web app." A type file that changed since the proposal is a 409 `stale`.
- It answers `{ op, note_id, version }` for a note, with the note's new version (for a delete, the version that went to the trash), and `{ op, type_name }` for a type, where `op` is `type_change` or `type_create`.
- Anything that is not a change the chat proposes is a 400.

## API

Settings > AI uses these routes. Every route under `/api/integrations/ai` needs an `admin` token or the web app's session; `GET /api/ai/models` and `GET /api/ai/providers` need `read`.

| Method and path | Does |
|---|---|
| `GET /api/integrations/ai` | Each provider's state: `configured`, the default `model`, the ChatGPT `account`, and for Anthropic and OpenAI the API `url` (null for the official API) and `has_key`, and for OpenAI the `api` its calls use, `responses` or `chat_completions` (always `responses` without a URL); the `default_provider`; the chat `system_prompt` in use, the shipped `default_system_prompt`, and `system_prompt_custom`; and `confirm`, `{ notes, archive, delete, types }`, with every switch filled |
| `PUT /api/integrations/ai/settings` | Saves `{ default_provider?, models?: { anthropic?, openai?, chatgpt? }, system_prompt?, confirm?: { notes?, archive?, delete?, types? }, urls?: { anthropic?, openai? }, openai_api? }`; an empty string clears a value, an empty `system_prompt` means the default, an empty URL the official API, and an empty `openai_api` the Responses API. Another `openai_api` than `responses` or `chat_completions` is a 400. A `confirm` sent changes only the switches it names. A `system_prompt` over 32,000 characters is a 400. A URL the rules above refuse is a 400 `invalid_url`, and nothing is saved |
| `PUT /api/integrations/ai/{anthropic\|openai}/key` | Stores `{ key }`; an empty key deletes it |
| `POST /api/integrations/ai/{provider}/test` | Sends the test request to `{ model }` or the default model, and answers `{ ok, model, ms, text, api, error }`. On OpenAI with a custom URL it detects and saves the API type as above, and `api` names the one the answer came over; otherwise `api` is null |
| `POST /api/integrations/ai/chatgpt/start` | Answers `{ auth_url }`, the ChatGPT sign-in page |
| `POST /api/integrations/ai/chatgpt/finish` | Takes `{ url }`, the pasted address, and stores the sign-in; a refused sign-in is a 400 with the reason |
| `POST /api/integrations/ai/chatgpt/disconnect` | Revokes and deletes the ChatGPT sign-in |
| `GET /api/ai/models?provider=` | The provider's models now, `{ provider, model, models: [{ id, display_name }] }`; without `provider`, the default one |
| `GET /api/ai/providers` | The chat panel's picker in every client: `{ default_provider, providers: { anthropic, openai, chatgpt: { configured, model } }, confirm }`. It holds no account, no system prompt, no API URL and never a key or token |
