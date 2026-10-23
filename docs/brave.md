# Web search with Brave Search

The AI chat panel can search the web through the [Brave Search API](https://brave.com/search/api/), with an API key from your own Brave account. Without a key, the model is not offered web search at all.

## Add the key

1. Get an API key from the [Brave Search API dashboard](https://api-dashboard.search.brave.com/). The free plan's limits are enough for occasional chat searches.
2. In the web app, open Settings > Integrations > Brave Search.
3. Paste the key into API key and select Save. The field then shows "set"; the key is never shown again.
4. Select Test. TheVault searches once for "TheVault" and shows how many results came back, or Brave's error, such as an invalid key.

To replace the key, paste another and save. Remove key deletes it, and the chat stops offering web search from the next question on.

The card is in the web app only. The key is stored encrypted in the server's secrets store as `brave_api_key`, so the server needs its secrets key file.

## What a search sends

A `web_search` call sends Brave the query the model wrote and the number of results it wants, with the key in the `X-Subscription-Token` header. Nothing else from the vault goes to Brave. Brave's own data policy applies to the queries it receives.

Each search writes one line to the server log, without the query:

```text
brave: count=5 results=5 outcome=ok ms=310
```

`outcome` is `ok` or `error:<kind>`: `rate_limit` when Brave answered 429, `auth` for 401, 403 and 422 (usually a missing or invalid key), `timeout`, `unavailable` or `provider`.

[AI providers](ai.md#web-search) describes the tool as the model uses it.

## API

Every route needs an `admin` token or the web app's session.

| Method and path | Does |
|---|---|
| `GET /api/integrations/brave` | `{ configured }`: whether a key is stored. Never the key |
| `PUT /api/integrations/brave/key` | Stores `{ key }`, trimmed; an empty key deletes it. Answers `{ configured }` |
| `POST /api/integrations/brave/test` | Searches once for "TheVault" and answers `{ ok, count, ms, error }`, `error` being null or `{ kind, message }` |

The secret `brave_settings` may hold `{ "endpoint": "http://127.0.0.1:1234" }` to send searches to another host than `https://api.search.brave.com`. The tests use it to reach a fake; nothing in the UI sets it.
