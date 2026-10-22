---
id: '122'
title: ./dev passes no stdin to the container, so piping into a command run through
  it reads nothing
blocked_by: []
status: done
verified: verified
---

# ./dev passes no stdin to the container, so piping into a command run through it reads nothing

Found during ticket 118. `echo hello | ./dev cat` prints nothing: ./dev adds -i only when both stdin and stdout are terminals, so `printf pw | ./dev target/debug/vault init ... --password-stdin` or `./dev vault put --type task <<EOF` get empty stdin. Ticket 118's test.py works around it by writing stdin to a file and redirecting inside `./dev sh -c`. Adding -i whenever stdin is not a terminal would fix it. Also, stopping the docker client of a `./dev` server leaves the container running, since nothing names or stops it.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:15Z — wil

Claimed.

### 2026-10-22T02:12:27Z — wil

Done on ticket/138 (efa00b3). ./dev always passes -i (adds -t when stdin and stdout are terminals); the client runs in the background with stdin handed over on fd 3, the container is named thevault-dev-<pid>-<random> and runs under --init. TERM/INT/HUP to ./dev are forwarded with docker kill --signal (a second signal removes it); an EXIT trap removes the container if the client dies first; only SIGKILL to ./dev leaves it. Observed: 'echo hello | ./dev cat' prints hello; here-string read works; 'printf pw | ./dev ... vault init /tmp/v --password-stdin' created the vault and key; exit codes pass through (sh -c 'exit 7' -> 7); TTY mode via script(1) reads typed input; './dev just server-run /tmp/sv 127.0.0.1:0' stopped 1s after kill -TERM of ./dev with no container left; TERM/INT/HUP on './dev sleep 300' exit 143/130/129 with no container left; SIGKILL of the docker client leaves no container. shellcheck clean, ./dev just check passes. Left alone: crates/cli/skill/thevault/test.py still uses its stdin-file workaround (out of scope for this worktree); it can drop it now.

### 2026-10-22T02:12:27Z — wil

Verified set to verified.

### 2026-10-22T02:12:27Z — wil

Status set to done.
