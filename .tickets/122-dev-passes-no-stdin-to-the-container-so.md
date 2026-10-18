---
id: '122'
title: ./dev passes no stdin to the container, so piping into a command run through
  it reads nothing
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# ./dev passes no stdin to the container, so piping into a command run through it reads nothing

Found during ticket 118. `echo hello | ./dev cat` prints nothing: ./dev adds -i only when both stdin and stdout are terminals, so `printf pw | ./dev target/debug/vault init ... --password-stdin` or `./dev vault put --type task <<EOF` get empty stdin. Ticket 118's test.py works around it by writing stdin to a file and redirecting inside `./dev sh -c`. Adding -i whenever stdin is not a terminal would fix it. Also, stopping the docker client of a `./dev` server leaves the container running, since nothing names or stops it.

## Worklog
