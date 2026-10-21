---
id: '138'
title: Cache the dev container image between CI jobs and runs, so each job skips the
  ~9 minute rebuild
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# Cache the dev container image between CI jobs and runs, so each job skips the ~9 minute rebuild

Every CI job that uses ./dev rebuilds docker/dev.Dockerfile from scratch (~9 min on ubuntu-24.04, seen in run 39524940666). Options: docker buildx with the GitHub Actions cache backend (type=gha) keyed by the Dockerfile hash that ./dev already uses for its tag, or push the dev image to GHCR as ghcr.io/wiltaylor/thevault-dev:<hash> and pull it. Measure before and after on release.yml, android-release.yml and the smoke job.

## Worklog
