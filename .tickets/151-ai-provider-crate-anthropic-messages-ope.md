---
id: '151'
title: 'AI provider crate: Anthropic Messages, OpenAI Responses (API key and ChatGPT
  plan), SSE, models, credentials'
plan: ai
blocked_by: []
status: todo
verified: unknown
---

# AI provider crate: Anthropic Messages, OpenAI Responses (API key and ChatGPT plan), SSE, models, credentials

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md (read it whole; this ticket is its 'crates/ai (ticket A1)' section and decisions D1 to D7, D9 to D12). Research: /home/wil/dev/TheVault/.scratch/research/llm-providers.md.

Build crates/ai as the spec describes: neutral request, message (D10) and event types; AiError with the kinds listed; one incremental SSE parser used by an async and a blocking transport; the Anthropic Messages client (anthropic-version 2026-10-21, no sampling parameters, no fallbacks parameter, adaptive thinking only when effort is set, output_config.format for schemas, strict tools, eager_input_streaming on streamed tools, schema validation of every tool input, no tools on max_tokens or refusal); the OpenAI Responses client used with an API key and with a ChatGPT plan token under D4's restrictions; live model listing per D6; Credentials::resolve with D9's single-flight refresh that writes the rotated pair back, invalid_grant handling and the one bell notification; Settings load and save for ai_settings including the endpoints test seam; trail_line per D12. Never use Codex CLI internals (D2).

Add to crates/testkit: fake_anthropic and fake_openai fixture servers (port 0) replaying scripted SSE streams and recording requests, able to answer refusal, 429 with retry-after and mid-stream failure; and a fake SIWC auth server (token, JWKS, openid-configuration, revocation) that signs ID tokens with a test key. Ticket A2 uses the auth fake for sign-in.

Acceptance criteria:
- Unit tests of the SSE parser with frames split at every byte boundary.
- Against the fakes: Anthropic streamed text, a tool call whose input arrives in several input_json_delta chunks, a schema-constrained reply, a refusal with category, 429 retried then OK; recorded requests carry anthropic-version 2026-10-21 and never temperature, top_p, top_k or fallbacks.
- OpenAI with an API key: text, function call, refusal content, response.failed. ChatGPT plan: requests carry store false and stream true and never max_output_tokens or previous_response_id; a plan request that ends without response.completed is an error; subscription_sharing_usage_limit_exceeded maps to rate_limit.
- provider_state round trip: a second request on the same provider and model replays thinking blocks with signatures (Anthropic) and reasoning items (OpenAI) verbatim; on a different model they are dropped.
- Refresh: an access token under 5 minutes from expiry is refreshed once even when 8 threads resolve at the same time; the rotated refresh token is stored; invalid_grant deletes the tokens, keeps client_id and host_id, notifies once.
- Model lists for all three shapes, ChatGPT filtered to visibility list.
- No key or token appears in any Debug output, error message or trail line (test it).
- ./dev just check passes.

## Worklog
