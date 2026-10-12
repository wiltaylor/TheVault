---
id: "33"
decision: D30
---

# Handler host API

## Question
Q29: What may handlers do?

## Answer
Handlers may query the vault and edit other notes through the normal save path, so their events fire and D9's depth limit applies. HTTP is allowed only in `after_*` handlers and scheduled tasks, under the domain allowlist.

## Why
Rollups need cross-note writes. Keeping HTTP out of sync handlers prevents slow APIs freezing saves.

## Consequences
Handler edits are attributed in history to the handler, not the user or token.
