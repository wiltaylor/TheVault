---
id: '106'
title: The type editor rewrites a type file from its model, so saving drops an attribute
  the reader ignored with a warning (ticket 100); it says so, but could keep the unknown
  items instead
blocked_by: []
status: done
verified: verified
---

# The type editor rewrites a type file from its model, so saving drops an attribute the reader ignored with a warning (ticket 100); it says so, but could keep the unknown items instead

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:01Z — wil

Claimed.

### 2026-10-22T02:47:36Z — wil

Branch ticket/103, commit b32676c. schema keeps ignored attributes as extra on TypeDef, FieldDef, Template, Tab and Slot; GET /api/schema lists them as extra {name: WCL text}; typeModel writes them back after known attributes (one the editor now writes itself is written once). Warning text now says saving keeps them. Tests: schema unknown_attributes_are_kept..., server schema_changes extra asserts, defs.test.ts round trip, e2e type-editor 'saving keeps attributes...'.

### 2026-10-22T02:47:36Z — wil

Verified set to verified.

### 2026-10-22T02:47:37Z — wil

Status set to done.
