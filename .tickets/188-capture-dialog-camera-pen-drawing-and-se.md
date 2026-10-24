---
id: '188'
title: 'Capture dialog: camera, pen drawing and Send to an agent'
plan: capture
blocked_by: []
status: todo
verified: unknown
---

# Capture dialog: camera, pen drawing and Send to an agent

Spec: /home/wil/dev/TheVault/.scratch/plans/capture/SPEC.md, K1 (web), K2, K3, K5. Acceptance: web component tests for the Camera button (file input with capture on a phone, the getUserMedia preview with a mocked stream on a desktop, hidden with no camera), the Pen surface (Done adds an ink block after the text, Discard adds nothing, a drawing removable before saving), Send to (lists Assistant and enabled agents, remembers the last choice, Save writes the agent: call link with a fresh call id after the body and toasts); a Playwright e2e that captures text with a drawing and Send to Assistant and finds the saved note's ink block and call link. Targeted tests only.

## Worklog
