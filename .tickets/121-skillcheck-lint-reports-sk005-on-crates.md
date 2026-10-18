---
id: '121'
title: skillcheck lint reports SK005 on crates/cli/SKILL.md because the skill name
  thevault differs from its directory cli
blocked_by: []
status: done
verified: verified
---

# skillcheck lint reports SK005 on crates/cli/SKILL.md because the skill name thevault differs from its directory cli

Found during ticket 118. `uvx --from pytest-skillcheck skillcheck lint crates/cli` fails with: error SK005 name is 'thevault' but the directory is 'cli'. SK005 is hard-coded in skillcheck's lint.py with no suppression. The name thevault is the contract of `vault skill` (crates/cli/tests/cli.rs asserts the document starts with name: thevault) and ticket 118 says keep it; the directory is the cli crate. Options: rename the skill to cli, move SKILL.md and test.py into a crates/cli/thevault/ directory and include_str! it from there, or accept the finding. It also warns SK010 (over 500 body lines); ticket 118 forbids splitting the file.

## Worklog

### 2026-10-18T04:47:26Z — wil

Fixed on main: SKILL.md and test.py moved to crates/cli/skill/thevault/; include_str! updated; skillcheck lint: 0 errors, SK010 length warning accepted by the user.

### 2026-10-18T04:47:26Z — wil

Verified set to verified.

### 2026-10-18T04:47:26Z — wil

Status set to done.
