---
id: '198'
title: 'Agents use skills: permission, the Available skills prompt section, use_skill,
  read_skill_file and install into the browser box'
plan: skills
blocked_by: []
status: todo
verified: unknown
---

# Agents use skills: permission, the Available skills prompt section, use_skill, read_skill_file and install into the browser box

Spec: /home/wil/dev/TheVault/.scratch/plans/skills/SPEC.md, S6, S7 (AI), S8 (agents), against S3's storage (ticket 1, in parallel; agree on the writer or api helpers for listing and reading skills). Acceptance: server tests against FakeAnthropic that the prompt lists only offered valid skills with name and description, use_skill answers the body and file list, read_skill_file answers text, an image part and metadata and refuses paths outside the skill, permissions.skills limits the list and the tools, unattended runs are offered the read tools, install_skill_in_browser copies the files into the fake exec service only with the browser permission, allowed-tools grants nothing; the Permissions tab Skills section (web test). Targeted tests only (no release yet).

## Worklog
