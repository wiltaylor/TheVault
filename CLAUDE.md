# TheVault

A single-user, self-hosted note app: Rust server, SolidJS 1.9 web client, `vault` CLI. Vault data is stored as WCL files.

## Source of truth for what to build

- `docs/plans/thevault-prd/SPEC.md` is the implementation specification. Build what it says.
- `docs/plans/thevault-prd/PRD.md` holds the user-facing requirements and the two grammar appendices (inline markup, query language).
- `docs/plans/thevault-prd/MAP.md` lists every decision (D1 to D42). Cite decision ids in commit messages when a change implements one.
- `docs/vault-format.md` defines the WCL vocabulary of every vault file once ticket 02 lands. It overrides the illustrative example in SPEC.md.
- Never edit anything under `docs/plans/`. A spec problem is a parked ticket, not an edit.

## Run every command through the dev container

The host has no Rust or Node toolchain for this project. Run every build, test, lint, format and run command through `./dev` from the root of your checkout or worktree:

```bash
./dev cargo test -p store
./dev pnpm -C web test
./dev just check
```

`./dev` mounts the current directory at the same path, runs as your user, and shares cargo and pnpm caches between containers. It uses host networking, and several workers run tests at the same time, so:

- Every test server, fixture HTTP server and Playwright target binds port 0 and reads back the port it got. Never hard-code a port.
- Every test vault lives in a fresh temporary directory. Never write into the repository or `$HOME` from a test.

`./dev just check` is the full gate: formatting, clippy with warnings denied, every Rust test, and the web typecheck and tests. Run it before every commit. Ticket 01 creates the `justfile`.

## Pinned upstreams

WCL and wscript come from GitHub as git dependencies pinned to a commit. Do not switch them to path dependencies or float them to a branch.

```toml
wcl_lang = { git = "https://github.com/wiltaylor/wcl.git", rev = "0531228286cb19d5b73daf7de23e79790a5e15d4" }
wscript  = { git = "https://github.com/Configweave/wscript.git", rev = "ddb6d9fedc77e68736a1281eaf8a122250f40aa3" }
```

## Answers to the spec's open questions

SPEC.md ends with OQ-S1 to OQ-S5. Build against these answers:

- OQ-S1: ticket 02 defines the vocabulary in `docs/vault-format.md`. Every other ticket follows that file.
- OQ-S2: the secrets key is a 32-byte key file outside the vault, at `$XDG_CONFIG_HOME/thevault/<vault-id>.key`, created by `vault init`. `VAULT_SECRET_KEY_FILE` overrides the path. `vault backup` leaves the key out unless `--include-key` is passed, and says so.
- OQ-S3: `vault init` sets the first password, prompting on a terminal or reading `--password-stdin`. `vault passwd` changes it. Passwords are stored as Argon2id hashes in `state.db`.
- OQ-S4: the inline colour palette is `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink` and `red`.
- OQ-S5: no sharding. Notes stay at `notes/<type>/<ULID>.wcl` as D22 says.

## Code layout

- `crates/<component>` holds one crate per component in the SPEC.md components table: `store`, `schema`, `writer`, `index`, `state`, `query`, `script`, `scheduler`, `api`, `cli`.
- `crates/server` holds the server binary. `crates/cli` builds the `vault` binary.
- `crates/testkit` holds the S1 helper that starts a server on a temporary vault and a random port.
- `web/` holds the SolidJS client, built with Vite and pnpm.
