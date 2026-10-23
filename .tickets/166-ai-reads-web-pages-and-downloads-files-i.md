---
id: '166'
title: AI reads web pages and downloads files into assets, with a guard against local-network
  addresses
plan: ai
blocked_by: []
status: todo
verified: unknown
---

# AI reads web pages and downloads files into assets, with a guard against local-network addresses

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D27 (with D19, D22, D23).

Add the guarded fetcher, the fetch_url and download_asset chat tools, the downloads confirmation switch with proposal and Apply through /api/ai/apply, the panel steps and cards, the chat prompt update, the fake web server, and docs.

Acceptance criteria:
- crates/ai or crates/api tests: the address rule refuses 127.0.0.1, 10.0.0.1, 169.254.1.1, ::1, an IPv4-mapped loopback and a redirect to 127.0.0.1; fetch_private lifts it; more than 5 redirects is refused.
- crates/server tests against the fake web server (with fetch_private): fetch_url on HTML returns the title and readable text without script or nav content and with links; a PDF returns its text; an image becomes an image part; a 6 MB text is cut; download_asset with the switch on emits a download proposal and stores nothing, Apply stores the asset and returns its id; with the switch off it stores at once and emits applied; over 25 MB is refused; a read-scope caller always gets a proposal.
- Web component tests: the Read and download steps, the download proposal card with Apply, the new switch in Settings > AI.
- ./dev env CI=true just check passes.

## Worklog
