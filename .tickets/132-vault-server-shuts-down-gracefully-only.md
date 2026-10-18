---
id: '132'
title: vault-server shuts down gracefully only on SIGINT (tokio ctrl_c); SIGTERM from
  docker stop, systemd or Kubernetes kills it without ending event streams or draining
  requests. The Docker image works around it with STOPSIGNAL SIGINT.
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# vault-server shuts down gracefully only on SIGINT (tokio ctrl_c); SIGTERM from docker stop, systemd or Kubernetes kills it without ending event streams or draining requests. The Docker image works around it with STOPSIGNAL SIGINT.

## Worklog
