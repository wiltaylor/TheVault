---
id: '132'
title: vault-server shuts down gracefully only on SIGINT (tokio ctrl_c); SIGTERM from
  docker stop, systemd or Kubernetes kills it without ending event streams or draining
  requests. The Docker image works around it with STOPSIGNAL SIGINT.
blocked_by: []
status: done
verified: verified
---

# vault-server shuts down gracefully only on SIGINT (tokio ctrl_c); SIGTERM from docker stop, systemd or Kubernetes kills it without ending event streams or draining requests. The Docker image works around it with STOPSIGNAL SIGINT.

## Worklog

### 2026-10-18T09:03:08Z — /work

Claimed.

### 2026-10-18T09:12:02Z — wil

Verified set to verified.

### 2026-10-18T09:12:02Z — wil

Status set to done.

### 2026-10-18T09:12:02Z — wil

SIGTERM and SIGINT share shutdown() in crates/server/src/main.rs via tokio::signal::unix. Before this, SIGINT also left state.db-wal non-empty because writer Arcs outlive serve() in scheduler/handler threads, so the server now calls State::checkpoint (wal_checkpoint TRUNCATE) after serve returns; the WAL file stays but is 0 bytes. Tests: crates/server/tests/events.rs an_open_stream_does_not_stop_the_server_shutting_down_on_{sigint,sigterm}. Dockerfile STOPSIGNAL SIGINT left as is.
