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

### 2026-10-24T04:21:36Z — wil

Design change before work started (user, 2026-10-24): the Available skills list goes at the END of the system text, as a per-thread snapshot taken on the first turn and kept unchanged; typing /update-skills in the chat box refreshes the thread's snapshot (turn request update_skills: true, no user message, a fixed short note to the model, a 'Skills updated (n)' notice in the panel). SPEC S6 rewritten. Extra tests: the section is last; a skill added mid-thread does not appear until /update-skills; /update-skills replaces the snapshot and shows the notice in the panel and the Chat tab.

### 2026-10-24T04:44:56Z — wil

Ticket 196 helpers: Writer::skills(), valid_skills(), skill(name) -> Skill{name, md: schema::SkillMd{name, description, license, compatibility, metadata, allowed_tools, extra, body, allowed_tools_list()}, files: Vec<SkillFileInfo{path,size,media_type,text}>, size, issues, valid()}; Writer::skill_file(name, path) -> bytes (confined); skill_text(name, path) (2 MiB cap, UTF-8); writer::skill_media_type(path). FORMAT_VERSION is now 3.
