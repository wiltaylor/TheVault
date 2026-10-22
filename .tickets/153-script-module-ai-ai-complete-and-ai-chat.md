---
id: '153'
title: 'Script module ai: ai::complete and ai::chat with JSON output and a run-log
  trail'
plan: ai
blocked_by:
- '151'
status: todo
verified: unknown
---

# Script module ai: ai::complete and ai::chat with JSON output and a run-log trail

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md (read it whole; this ticket is its 'ai script module (ticket A3)' section and D11, D12, D13). Builds on crates/ai from ticket 151. Model the module on crates/script/src/http.rs (blocking client built outside the async runtime, deadline-bounded requests).

Add the ai module to After and Task runs, update the kind table in crates/script/src/lib.rs, register it in the embedded host returning unavailable, and add a ## ai section to docs/scripts.md after secrets in the existing format. The module reads credentials through crates/ai itself; scripts list neither the ai_* secrets nor provider hosts. AiError reaches the script as a struct with kind, message, provider, model and category; if wscript cannot carry a struct in Err, park the ticket with the reason rather than flattening it to a string.

Acceptance criteria:
- crates/script tests against the testkit fakes: ai::complete returns text and token counts; with schema set, Reply.json is the parsed object; ai::chat sends the history in order; options override ai_settings defaults; a refusal returns Err with kind refusal and the category; not configured returns kind not_configured.
- Every call appends exactly one D12 trail line to the run log with outcome ok, refused or error, and the prompt text never appears in it.
- A call that would outlast the run deadline ends with the timeout fault, as http does.
- A Before handler using ai fails to compile; the embedded host returns kind unavailable.
- A scheduled-task test runs a task that calls ai::complete and records the trail in task_runs.
- ./dev just check passes.

## Worklog
