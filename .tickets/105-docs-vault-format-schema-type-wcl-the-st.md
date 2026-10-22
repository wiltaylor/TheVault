---
id: '105'
title: docs/vault-format/schema/type.wcl (the structural schema check.sh uses) lacks
  the hidden, labels and accept field items and refuses unknown attributes, so it
  no longer matches the readers' rule from ticket 100
blocked_by: []
status: done
verified: verified
---

# docs/vault-format/schema/type.wcl (the structural schema check.sh uses) lacks the hidden, labels and accept field items and refuses unknown attributes, so it no longer matches the readers' rule from ticket 100

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:01Z — wil

Claimed.

### 2026-10-22T02:47:36Z — wil

Branch ticket/103, commit b32676c. type.wcl declares hidden, labels, accept, option_labels and Tab.when; specimen type uses each plus of = date_or_datetime (notes/specimen.wcl schema updated). WCL has no warning level, so check.sh runs wcl check --json through tolerated.mjs: undeclared attributes in a type file (type, field, template, tab, slot, fixed blocks; not handlers) print as warnings. New fixtures tolerated/newer-type.wcl (passes with 6 warnings) and invalid/type-wrong-form.wcl (rejected). ./dev env WCL=.scratch/wcl/bin/wcl docs/vault-format/check.sh exits 0; a schema test loads the same fixture with 6 warnings.

### 2026-10-22T02:47:36Z — wil

Verified set to verified.

### 2026-10-22T02:47:36Z — wil

Status set to done.
