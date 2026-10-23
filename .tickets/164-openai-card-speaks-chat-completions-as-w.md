---
id: '164'
title: OpenAI card speaks Chat Completions as well as Responses, detected by Test
plan: ai
blocked_by: []
status: todo
verified: unknown
---

# OpenAI card speaks Chat Completions as well as Responses, detected by Test

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D25 (with D10, D11, D19, D24).

Add endpoints.openai_api, a Chat Completions client behind the existing request and event types (streaming, tool calls, JSON schema with a json_object fallback, images after tool messages, refusals, usage), Test's detection and saving of the API type, the API type select on the OpenAI card, the FakeOpenAi Chat Completions mode, and docs.

Acceptance criteria:
- crates/ai tests against FakeOpenAi in Chat Completions mode: streamed text; a tool call whose arguments arrive in several chunks; a schema reply, and the json_object fallback after a 400 for json_schema; finish_reason length and content_filter; usage counted; an image tool result becomes a tool message plus a user message with image_url parts; replaying provider_state is exact; requests never carry temperature, top_p or strict; the ChatGPT plan and the official OpenAI URL always use Responses.
- crates/server tests: Test on a Chat-Completions-only fake detects it, saves openai_api and reports it; a chat turn with search_notes then an answer works over Chat Completions; a script ai::complete works over it.
- Web component tests: the API type select shows with a custom URL and saves with the card; a Test result naming the detected type shows.
- ./dev just check passes.

## Worklog
