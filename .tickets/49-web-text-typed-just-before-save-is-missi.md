---
id: '49'
title: 'web: text typed just before Save is missing from the saved WCL'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: text typed just before Save is missing from the saved WCL

Found by the orchestrator's clean e2e run on main e42c704 (idle machine, 67/68 passed). e2e/editor.spec.ts:51 'typing each mark through shortcuts and the toolbar saves the expected WCL' fails: the expected paragraph ends with ' 2\*3' but the stored text stops after the mention, so the final typed characters were not saved. Decide from the code whether Save does not flush pending editor input (a data-loss bug: fix the editor so Save, and navigating away, always persist the current document) or the test saves before input settles (fix the test to wait on the editor state, not a timer). Acceptance: the root cause is named in the worklog; if it is the editor, a new Playwright test types and saves immediately and the file holds every character; editor.spec.ts passes 10 runs in a row with --repeat-each=10.

## Worklog

### 2026-10-12T19:33:40Z — /work

Claimed.

### 2026-10-12T19:43:36Z — wil

Root cause: the editor, not Save and not the test. MentionPicker.finish refocused the editor with TipTap's focus(), which on desktop Chromium calls view.focus() inside requestAnimationFrame (@tiptap/core commands/focus.ts). The picker's input was already removed, so keys typed before the next frame went to <body> and were lost; under load rAF runs late, which is why the orchestrator's run saw ' 2*3' missing after the mention. Save itself reads state updated synchronously by onUpdate and was fine. Fix: MentionPicker and ContentEditor's chain()/link-Escape call editor.view.focus() synchronously first. New e2e test 'text typed straight after choosing a mention and saved at once is all in the file' slows rAF to 300 ms, presses Enter, types and saves with no waits; it failed before the fix (text stopped after the mention) and passes after. editor.spec.ts --repeat-each=10: 50 passed. just check passed. Full e2e: first run 67/69 with definitions.spec:35 and date-views.spec:59 timing out at load avg ~72 (neither touches the editor; both pass 6/6 alone), second run 69/69 — ticket 48 territory.

### 2026-10-12T19:43:36Z — wil

Verified set to verified.

### 2026-10-12T19:43:36Z — wil

Status set to done.

### 2026-10-12T19:45:41Z — wil

Merged into main at 21c405b (fast-forward after merging main f69a577 into the branch; just check and editor.spec.ts green after the merge).
