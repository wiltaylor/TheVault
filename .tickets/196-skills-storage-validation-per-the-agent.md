---
id: '196'
title: Skills storage, validation per the Agent Skills spec, sync, and the API with
  zip import and export
plan: skills
blocked_by: []
status: done
verified: verified
---

# Skills storage, validation per the Agent Skills spec, sync, and the API with zip import and export

Spec: /home/wil/dev/TheVault/.scratch/plans/skills/SPEC.md, S1, S2, S3, S7 (writes), S8 (format). Acceptance: validation tests for every frontmatter constraint (name length, characters, hyphens, folder match, description length, compatibility length, metadata map of strings, unknown keys as warnings, missing frontmatter); store tests for the opaque skills/ tree, limits and path confinement; server tests for each route and its scope, import of a folder zip and a root-SKILL.md zip, a refused existing name and replace=true, an invalid skill refused before anything is written, export round-trip; sync tests that a skill with a binary asset reaches a device; activity entries. Targeted tests only (no release yet).

## Worklog

### 2026-10-24T04:44:56Z — wil

Verified set to verified.

### 2026-10-24T04:44:56Z — wil

Status set to done.
