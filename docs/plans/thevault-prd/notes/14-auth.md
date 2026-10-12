---
id: "14"
decision: D11
---

# Auth and exposure

## Question
Q14: How are browser and agents authenticated?

## Answer
Browser: password login with a session cookie. Agents: API tokens with scopes `read`, `write`, `admin` (admin covers schemas, templates and scripts). Designed for LAN or tailnet. Internet exposure is supported behind a reverse proxy.

## Why
Scopes let an agent edit content without touching automation.

## Consequences
Token storage and rotation details are an accepted unknown.
