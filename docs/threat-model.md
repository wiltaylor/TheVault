# Untrusted input and agent authority

TV-143. Rules: the project wiki `security-decisions`, re-read on 2026-11-11 after TV-137 landed.
This page describes deterministic server enforcement, not resistance of a model
to persuasive text. `FakeAnthropic` deliberately requests forbidden actions.
No network provider, real credential or production vault is used.

## Principals and boundary

Text is not a principal. Email senders, other people's calendar organisers, web
sites, shared files, peers, devices and other agents cannot grant an in-app
agent permissions by writing instructions. A skill's `allowed-tools` is also
not a grant. User sessions, user tokens, device tokens and external agent tokens
are distinct authorities. A foreground run is not equivalent to a trigger run.

The matrix's active baseline is an agent with `get_note`, `fetch_url` and
`use_skill` only. The fake provider first reads the source successfully, then
attempts the column's operations. Every attempted existing-tool operation must be unoffered
and return an authority error. Columns without built-in tools inspect a maximal
agent's offered tool list instead. Outbox plus the complete agents/ and skills/ trees must remain unchanged
(including added or removed files and empty directories). The zip fixture is
imported before taking the tree snapshots.
This is a least-privilege non-escalation test, not evidence for every permitted
tool's internal confirmation, Google or send-limit branch. Existing dedicated
suites below test those branches with the tools enabled.

Content rows use the **normalised post-import note-body boundary**, with the
source named in the note title. They do not simulate SMTP, SMS, git transport,
device upload or attachment extraction. Web uses a real `fetch_url` against
`FakeWeb`. Skills use a multipart zip import followed by real `use_skill`.
Agent output additionally places the attack in the fake assistant response.
Transport identity and import escaping remain separate tests. A reader must
not interpret identical normalised fixtures as independent importer coverage.
Device rows also use a redeemed pairing token. External-token HTTP assertions
use redeemed install-code credentials, but their content baseline uses the
owner's foreground chat because TV-185 will prohibit external chat altogether.
The separate ignored TV-185 entry test covers both synchronous and background
chat and asserts that no provider was called.

## Matrix

Each entry names a rule card from `security-decisions`. `B` means active
least-privilege baseline. `P` means pending enforcement, not passing coverage.
Column names match `crates/server/tests/threat_model.rs`. Every row expands to
individually named libtest cases. The completeness test rejects missing rows,
columns and duplicate registrations when either axis is extended.

| Source | held_notes | send | calendar | gmail | code_secrets | agent_skills | tokens_pairing | integrations | browser_login | triggers_backfill |
|---|---|---|---|---|---|---|---|---|---|---|
| email | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| sms | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| message | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| call | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| calendar_event | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| web_page | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| shared_note | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| peer_git_sync | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| device_sync | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | HTTP TV-140 scope | HTTP TV-140 scope | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| external_agent_token | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*); HTTP TV-142 | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140; P TV-185 | HTTP TV-142 | HTTP TV-142 | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| agent_run_output | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| skill_zip | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |
| shared_attachment | TV-137 (active) | B (allowlist; TV-171 in ai_outbox/outbox_*) | B TV-136 | B (allowlist; TV-138 in gmail_actions) | B TV-135 | B TV-140 | N: no built-in agent tool | N: no built-in agent tool | B TV-160; P TV-161/TV-167 | B TV-140; HTTP TV-170 |

Attachments of incoming email inherit that email's source and gate. The
shared_attachment row represents files shared independently of email.

There are 132 active cases: 130 source/capability cells, one completeness
check and one granted-tool schema control. The 27 pending cases are 13
Login-grant proposal checks for TV-161, 13 signed profile checks for TV-167 and
one external-token chat-entry check for TV-185. The latter protects every
external-token capability. Ignore reasons name their implementation cards.

The TV-137 cells read a held target after each source's payload. Email and
non-note sources target an incoming email. SMS/message and device sources
target an incoming message, call targets a call, and calendar_event targets an
event organised by outside@example.com. These are four target-note kinds, not
13 independent write-path or transport tests. Git-sync gate coverage remains
in the dedicated writer tests, not in the normalised peer_git_sync fixture.

N pins the full offered built-in tool-name set of a maximal agent, so any new
tool needs explicit classification, rather than refusing an imaginary call. It does not restrict
owner-installed script tools. HTTP TV-142 cases use a legacy admin-scope agent
token seeded through state and exercise the admin-to-write cap. The kind guard
is redundant behind that cap and is detected only when both are removed.
HTTP TV-140 device cases exercise ordinary scope checks. Every backfill cell
also checks an ordinary admin user token gets session_required, which reaches
TV-170's guard rather than stopping at write scope. HTTP checks verify the
error code as well as status. Sending additionally matches its review-required
message. Current source/capability checks remain least-privilege baselines,
not enabled-tool internal-policy coverage.

Active browser cells configure a FakeBrowser while keeping the agent's browser
permission off, so refusal exercises that permission rather than box absence.
They additionally create a Login secret through the owner API,
check that metadata omits its password, and verify that the ordinary secret
reader cannot return it. The pending TV-161 test has an owner-valid grant as a
positive control before checking that an agent proposal still waits for Apply.
The pending TV-167 test compares two agents' server-selected signed browser
identities using schema-valid browser commands with no client profile selector.

## Rule-specific assertions and existing suites

- H (TV-137, now active): no agent reads an incoming note without an allow verdict.
  Release and reclassification are session controlled. The organiser rule for
  calendar events and fixed held-note notice are part of this boundary.
- S (TV-171, TV-142): drafting is allowed with the tool, sending requires Outbox
  review or explicit recipient/kind/cap/delay conditions. External agent tokens
  never bypass review. `ai_outbox`, `outbox_email`, `outbox_sms`.
- C (TV-136): invitations, attendee edits, RSVP and event deletes use the
  Outbox. `events`, `google_calendar`, `outbox_event`.
- G (TV-138): Gmail archive/trash needs foreground permission or user Apply,
  never a trigger or mention. Restore keeps the local note on Google failure.
  `gmail_actions`.
- K (TV-135): code/host/secret grants require session Apply, including tool-file
  bindings, skill scripts and type handlers. Shipped scripts are read-only.
  `ai_script_changes`, `ai_skill_changes`, `script_tools`.
- A (TV-139, TV-140): read/change type ceilings hold at propose and Apply.
  Agent and skill files require session/admin authority. `agents`, `auth`,
  `agent_files`, `ai_agents`, `ai_skills`.
- T and I (TV-142): external tokens are agents by stored kind, capped at write,
  and cannot mint tokens/codes or change integrations. `review_probes`, `auth`.
- B (TV-160): Login passwords/seeds are write-only and encrypted, separate from
  string secrets. `scheduler`. TV-161 adds per-agent names, owner-only widening
  and recreated-name invalidation. TV-167 adds server-selected profiles. These
  later grants/profile assertions must not be inferred from baseline refusal.
- R (TV-170): durable deduplicated bounded trigger jobs, conservative retries,
  owner-session-only previewed backfill. `agent_triggers`, `agent_capacity_retry`.
- TV-185: external agent tokens cannot drive an in-app agent or apply in its
  name. Those assertions remain pending until its implementation lands.

## Limits and accepted risks

The matrix is not a proof against all tool combinations or malicious installed
code. Browser profiles share a Linux user until TV-175. TV-137 intentionally
does not classify events organised by the user, even if guests edited the text
or an invitation forged that organiser. Explicitly granted foreground Gmail
actions and constrained sending remain permitted. The roadmap's future safety
rails are not silently substituted for current detailed security decisions.

Attempt limits (TV-141) and call-id replay/import guards (TV-134) are orthogonal
entry controls, covered by `auth_limits` and `agent_mentions`. Production
stopgap changes are operational decisions, not capabilities granted by content.
