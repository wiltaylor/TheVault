# Untrusted input and agent authority

TV-143. Rules: the project wiki `security-decisions`, read at revision 37.
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
attempts the column's operations. Every attempted operation must be unoffered
and return an authority error. Outbox and agent files must remain unchanged.
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
| email | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| sms | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| message | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| call | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| calendar_event | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| web_page | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| shared_note | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| peer_git_sync | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| device_sync | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| external_agent_token | P TV-137 | B TV-171/TV-142 | B TV-136/TV-142 | B TV-138 | B TV-135 | B TV-140; P TV-185 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170; P TV-185 |
| agent_run_output | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| skill_zip | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |
| shared_attachment | P TV-137 | B TV-171 | B TV-136 | B TV-138 | B TV-135 | B TV-140 | B TV-142 | B TV-142 | B TV-160; P TV-161/TV-167 | B TV-170 |

Attachments of incoming email inherit that email's source and gate. The
shared_attachment row represents files shared independently of email.

At this revision there are 118 active cases (117 baseline cells and one
completeness check) and 40 pending cases: 13 held-note cells for TV-137,
13 Login-grant proposal checks for TV-161, 13 signed profile selection checks
for TV-167 and one external-token chat-entry check for TV-185. The latter
protects every external-token capability, not just the two annotated groups.
Each pending case contains an executable secure-behaviour assertion and names
the implementing card in its `ignore` reason. On landing that card, remove the
corresponding ignores and run the matrix. A failure must become a security
finding rather than a product patch hidden in this test card.

Active browser cells additionally create a Login secret through the owner API,
check that metadata omits its password, and verify that the ordinary secret
reader cannot return it. The pending TV-161 test has an owner-valid grant as a
positive control before checking that an agent proposal still waits for Apply.
The pending TV-167 test compares two agents' signed browser identities despite
an attacker-supplied profile argument.

## Rule-specific assertions and existing suites

- H (TV-137): no agent reads an incoming note without an allow verdict.
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
