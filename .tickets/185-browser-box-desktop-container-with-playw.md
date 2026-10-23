---
id: '185'
title: 'Browser box: desktop container with playwright-cli and an exec service, compose
  profile and release image'
plan: browser
blocked_by: []
status: done
verified: verified
---

# Browser box: desktop container with playwright-cli and an exec service, compose profile and release image

Spec: /home/wil/dev/TheVault/.scratch/plans/browser/SPEC.md, B1, B2, B3, B8, B9. Acceptance: crates/browser-agent tests (auth, run with exit and output, the 64 KiB tail caps, a timeout killing the process group, 4 concurrent then 429, file reads confined to /work and /home/agent with symlink escapes refused, 25 MiB cap, listing, health); just browser-image-test passes on the host; compose-test checks the browser profile's config; release-train.yml builds and pushes the image; docs/install.md, env.example, docs/releasing.md. Targeted tests only (the full gate runs before the next release).

## Worklog

### 2026-10-23T09:32:38Z — wil

Verified set to verified.

### 2026-10-23T09:32:38Z — wil

Status set to done.
