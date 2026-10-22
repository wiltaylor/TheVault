# AI providers

TheVault can call large language models with your own credentials. Scripts call them through the `ai` module, and the web app's AI panel uses them to answer questions about the vault. The server makes every call itself: no third party sits between the vault and the provider.

This page covers the three providers, how to connect each in Settings > AI, what the ChatGPT plan allows, what data leaves the server, and the log line every call leaves.

## Providers

| Provider | What you need | Where requests go |
|---|---|---|
| Anthropic | An API key from the [Anthropic Console](https://console.anthropic.com/) | `https://api.anthropic.com/v1/messages` |
| OpenAI | An API key from the [OpenAI Platform](https://platform.openai.com/api-keys) | `https://api.openai.com/v1/responses` |
| ChatGPT | A ChatGPT plan, signed in with OpenAI's Sign in with ChatGPT | `https://api.openai.com/v1/responses`, with the plan's access token |

The two API-key providers bill your API account per token. The ChatGPT provider uses the allowance of your ChatGPT plan instead.

## Settings > AI

Settings > AI is in the web app only. The desktop and Android apps talk to their embedded server, which holds no AI credentials, so the section is absent there.

Each provider has a card with:

- Its credential: an API key field, or the ChatGPT sign-in.
- A model list, read live from the provider each time the card opens. No model id is built into TheVault, except that the Anthropic card preselects `claude-opus-5` when Anthropic lists it. The model you pick is that provider's default.
- A **Test** button. It sends a short "Reply with OK" request (16 output tokens at most) to the selected model and shows the result, such as `OK from claude-opus-5 in 840 ms`, or the error kind and message.
- **Use <provider> by default**, which makes it the provider scripts and the AI panel use when they do not name one. The first provider you connect becomes the default.

A script or the AI panel can still name another provider and model for one call.

### Add an API key

1. Open Settings > AI.
2. Paste the key into the Anthropic or OpenAI card's **API key** field and select **Save key**.
3. Pick a model from the list, then select **Test**.

The server stores the key encrypted and never sends it back. The field then shows `set` as its placeholder. To replace the key, paste another and save. **Remove key** deletes it.

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
- The AI panel sends the conversation and the results of the note tools the model calls, such as the notes it searches and reads.
- The Test button sends only "Reply with OK."

Nothing else from the vault is sent. Requests to OpenAI set `store: false`, so OpenAI keeps no stored response for later reuse. Each provider's own data policy applies to what it receives.

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

The error kinds are `not_configured`, `auth`, `rate_limit`, `refusal`, `invalid`, `timeout`, `unavailable` and `provider`.

A script's calls write the line to the run's log. The server's own calls, such as Test, write it to the server log. The prompt text never appears in it.

## Chat panel

The AI panel in the right sidebar answers questions about your notes and proposes edits to them. Open it with the AI tab, Ctrl+Shift+L (Cmd+Shift+L on a Mac), or Ask AI in the command palette (Ctrl+K). It uses the default provider and model from Settings > AI; the two menus at the top of the panel pick another for this conversation.

The panel shows only in the web app. The desktop and Android apps talk to their own embedded server, which holds no AI credentials.

### Ask a question

Type a question and press Enter (Shift+Enter adds a line). The answer streams in as the model writes it. When a note is open beside the panel, the model knows its id, type and title, so "this note" means that note.

To answer, the model calls note tools on the server. Each call shows as a step above the answer, such as “Searched for “roof””, with links to the notes it found or read. The answer links the notes it relies on; a click opens one in the active pane.

| Tool | Does |
|---|---|
| `search_notes` | Full-text search, at most 20 notes |
| `query_notes` | A query in the vault query language, at most 50 notes |
| `get_note` | Reads one note, body included; each text over 40,000 characters is cut with a marker |
| `backlinks` | The notes that link to a note |
| `propose_edit` | Checks an edit to one note and shows it to you; never writes |

A tool result over 100,000 characters is cut with a marker before it goes to the model. One question makes at most 12 calls to the model; after that the panel says it stopped, and asking again goes on.

Stop ends the answer at once: the server stops calling the model when the panel goes away. New conversation forgets the conversation. Conversations live in the browser tab only and are not saved in the vault.

### Apply an edit

When you ask for a change, such as "add eggs to my shopping list", the model proposes it. The panel shows a card with the changed fields and the lines of text removed and added:

- Apply writes the change as you, at the version the model read. If the note changed since, Apply fails and says so; ask again for a fresh proposal.
- Discard drops it. Nothing is written.

The model never writes a note itself.

### The system prompt

The **Chat system prompt** card in Settings > AI holds the instructions the panel sends the model before every conversation, for every provider. It shows the prompt in use and whether it is the **Default** or **Customised**:

- Edit the text and select **Save**. The next question uses it. The prompt can be at most 32,000 characters.
- **Reset to default** goes back to the prompt TheVault ships. It is disabled while the default is in use. Saving an empty prompt, or the default text unchanged, also resets it.

The server adds the open note's id, type and title after your prompt, so the editable text never holds them. The tool list reaches the model through the tool definitions, so a custom prompt does not need to repeat it. The default prompt also summarises the query language and how to link notes; keep those parts if you want the model to go on doing both well.

Scripts do not use this prompt: a script's `ai` call passes its own `system`.

### Refusals and errors

When the provider refuses a request, the answer stops where it was and the panel shows which model refused and the category the provider gave, such as `cyber`. TheVault never retries a refusal with another model. Other failures show the error message, for example a rate limit.

### The chat route

`POST /api/ai/chat` needs a `read` token or the web app's session. Applying a proposal uses the notes write route, which needs `write`. The body is `{ provider?, model?, messages, note_id? }`, where `messages` is the conversation so far ending with the new question. The answer is a stream of server-sent events, with a heartbeat comment every 15 seconds:

| Event | Data |
|---|---|
| `text` | `{ delta }`, the next piece of the answer |
| `tool` | `{ id, name, input }`, a tool call about to run |
| `tool_result` | `{ id, summary, is_error }`; `summary.notes` lists the ids and titles returned, never note bodies |
| `proposal` | `{ id, note_id, title, base_version, before, after, ops }`, an edit to apply with `PUT /api/notes/{note_id}` |
| `usage` | `{ provider, model, input, output }` for one model call |
| `refusal` | `{ provider, model, category, explanation, messages }`; the last event |
| `error` | `{ kind, message, messages }`; the last event |
| `done` | `{ stop, messages }`; the last event |

`messages` holds the turn's new conversation items, including the provider's own record of each answer. Send them back unchanged, after the earlier ones, with the next question. Each call to the model writes a trail line to the server log.

## API

Settings > AI uses these routes. Every route under `/api/integrations/ai` needs an `admin` token or the web app's session; `GET /api/ai/models` needs `read`.

| Method and path | Does |
|---|---|
| `GET /api/integrations/ai` | Each provider's state: `configured`, the default `model`, and the ChatGPT `account`; the `default_provider`; the chat `system_prompt` in use, the shipped `default_system_prompt`, and `system_prompt_custom` |
| `PUT /api/integrations/ai/settings` | Saves `{ default_provider?, models?: { anthropic?, openai?, chatgpt? }, system_prompt? }`; an empty string clears a value, and an empty `system_prompt` means the default. A `system_prompt` over 32,000 characters is a 400 |
| `PUT /api/integrations/ai/{anthropic\|openai}/key` | Stores `{ key }`; an empty key deletes it |
| `POST /api/integrations/ai/{provider}/test` | Sends the test request to `{ model }` or the default model, and answers `{ ok, model, ms, text, error }` |
| `POST /api/integrations/ai/chatgpt/start` | Answers `{ auth_url }`, the ChatGPT sign-in page |
| `POST /api/integrations/ai/chatgpt/finish` | Takes `{ url }`, the pasted address, and stores the sign-in; a refused sign-in is a 400 with the reason |
| `POST /api/integrations/ai/chatgpt/disconnect` | Revokes and deletes the ChatGPT sign-in |
| `GET /api/ai/models?provider=` | The provider's models now, `{ provider, model, models: [{ id, display_name }] }`; without `provider`, the default one |
