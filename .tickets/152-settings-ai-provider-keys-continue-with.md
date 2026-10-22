---
id: '152'
title: 'Settings > AI: provider keys, Continue with ChatGPT sign-in, model lists and
  Test'
plan: ai
blocked_by:
- '151'
status: done
verified: verified
---

# Settings > AI: provider keys, Continue with ChatGPT sign-in, model lists and Test

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md (read it whole; this ticket is its 'Settings > AI (ticket A2)' section and D6, D7, D8, D15, D16). Builds on crates/ai from ticket 151. Model the routes, pending-state handling and paste box on Google's (crates/api/src/google.rs, web/src/pages/Integrations.tsx GoogleCard).

Build the /api/integrations/ai routes and GET /api/ai/models (read scope), the SIWC start, finish and disconnect flow of D8 (redirect http://127.0.0.1:53683/auth/callback, dynamic_agent_client on first sign-in then the issued oaiapp id with id_token_hint and login_hint, host_id kept across sign-ins, ID token verified against JWKS, chatgpt.tokens.use.direct required, revocation on disconnect), the Test route, and web/src/pages/Ai.tsx as a Settings section in WEB_APP_ONLY. UI wording per OpenAI's guidelines: the button says Continue with ChatGPT, the connected state says Using ChatGPT plan as <account>, with a Manage usage link to https://chatgpt.com/settings/usage. Write docs/ai.md.

Acceptance criteria:
- crates/server test against the testkit fakes: start returns an authorize URL with every D8 parameter; finish with the pasted URL stores ai_chatgpt; a second start uses the issued client id with id_token_hint and the same host_id; a wrong state, a bad ID token signature, a wrong nonce and a missing chatgpt.tokens.use.direct scope are each refused with a clear 400; disconnect calls revocation and deletes the secret.
- Key PUT stores and an empty key deletes; GET /api/integrations/ai and every other response never contain a key or token (assert on the raw bodies).
- Test route returns ok with the model against each fake, and the error kind and message on failure.
- A read-scoped token can GET /api/ai/models and is refused on /api/integrations/ai.
- Web component tests for the AI section (key set state, model select from the list, Test result, the ChatGPT paste flow) and the section is absent in the app builds.
- ./dev just check passes.

## Worklog

### 2026-10-22T15:37:34Z — wil

Verified set to verified.

### 2026-10-22T15:37:34Z — wil

Status set to done.
