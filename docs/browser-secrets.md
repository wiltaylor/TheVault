# Login secrets

Settings > Secrets supports String secrets and Login secrets. Existing String
secrets still work as before. A Login stores a user name, password, optional
base32 one-time-code seed and optional HTTP(S) site origins. Login credentials
are encrypted in state.db with the existing key outside the vault. User name,
origins and has_totp are clear metadata, so listing never decrypts credentials.
A missing or wrong key prevents credential reads and replacement, not metadata
reads or deletion. Login secrets never
enter vault git sync. Backups carry the encrypted row, not decrypted credentials.
Keep the key separately unless you deliberately include it in a backup.

Only an owner session or an admin user token can create, replace or delete a
Login. Replacing it always requires the password again, including changes to
origins or the seed. The API returns metadata, never the password or seed.
Login values cannot be read by ordinary script-secret bindings or agent tools.

Origins use exact scheme, host and effective port matching, not subdomains or
suffixes. Hosts are lowercased and IDNA encoded, and default ports and one
trailing dot are removed. Wildcards and empty domain labels are refused.
IP literals are allowed. Paths, credentials, queries and fragments are refused.
No origins means any site, with a warning.

## Browser fill service

`browser_fill_secret {secret, field, target, session}` requests `username`,
`password` or `otp`. On the agent's Permissions tab, **Ask before entering a
secret** (`permissions.confirm.secrets`) defaults to on for both existing and
new agents. Only a signed-in owner session or ordinary admin user token can
change it. Agent proposals changing this switch always wait for owner Apply.
Sub-agents cannot weaken their parent's confirmation requirement.

With the switch on, a foreground call creates a held proposal in the thread.
It names the secret, field, target selector, browser session and canonical page
origin, never the value. A secret with no origin list shows an any-site warning.
Apply needs an owner session or ordinary admin user token. It reloads current
permissions and profile and rechecks the grant, revocation and origin. Navigation
to another origin since the proposal refuses the fill, even for an unrestricted
secret. Discard fills nothing. Metadata that matches a credential is refused
rather than published or silently changed.

Unattended and mention runs with the switch on refuse with
`confirmation_required`. With it off, calls fill directly through the same
authorized service, within the run's write-scope ceiling, including the scope
issued to a mention call. A model-supplied confirmation flag has no effect.
The internal authorized service is not an HTTP API available to agents. Install
updated apps before saving `secrets = false`: older apps refuse this new agent
file field. Leaving it on needs no migration or file rewrite.

That service checks the agent's Login grants and their current validity, reads
the page origin from the signed-profile box channel, and compares exact canonical
scheme, host and port. It computes OTP on the server using the same RFC 6238
helper as redaction. Values travel only in the request body, bridge stdin and
raw CDP messages. The box checks the origin again in the same document operation
that sets the input. Tool results are `filled` or a fixed refusal, never browser
diagnostics. Each attempt produces one value-free audit entry in each of the
server and run logs, with agent, secret name, field, origin and allowed/refused.
An origin not read because confirmation or permission failed is empty.

The bridge supports the pinned `@playwright/cli` 0.1.22 with `playwright-core`
1.64.0-alpha-1790635538000. Open a persistent Chromium session with
`playwright-cli open <url> --headed --persistent`. Use a CSS selector for `target`
and `default` or the CLI session name for `session`. Initially a session must
have exactly one top-level page. Frames, multiple tabs, non-HTTP(S) pages,
ambiguous selectors, and non-input elements are refused. Existing sessions
without the debugging listener must be reopened. Custom launch configurations
that remove the listener are not supported.

Only agents with a nonempty effective `permissions.browser_secrets` list get
the debugging launch configuration. Ordinary profiles get none. Chromium opens
a random loopback CDP port for the lifetime of that browser session. Its existing
debugging pipe belongs to the CLI daemon and cannot be shared safely by a separate
bridge, and Chromium cannot turn that listener on just for a fill. On the next
browser command after eligibility changes, the box closes that profile's old
processes and changes the configuration. Reset/delete also closes them. No CLI
command, including a supposedly value-free query, runs as part of a fill:
automatic CLI snapshots could otherwise persist a previously filled field.

Known limits for browser filling:

- R1: A password must be in the page to work. A misled agent with a shell in
  the browser box could reach it indirectly. This is assumed, not tested.
- R2: Prompt injection can make an agent sign in and act.
- R3: A Login with no origin list can be entered on a look-alike page.
- R4: Until TV-175 isolates agents, another agent's shell in the same box can
  attach to a loopback CDP listener and watch a live fill, not just read profile
  files. The signed identity does not isolate processes sharing one Linux user
  and network namespace. A grant removed during a turn may leave that turn's
  listener open until its next setup/command or profile reset.
- R5: After the value reaches the page, the site can read and transmit it.
  Later agent-requested snapshots, screenshots, tracing or shell commands can
  capture page values. The fill transport does not run those operations, but
  it cannot prevent them. Browser output redaction has the limits below.

## Browser output redaction

The server masks all stored Login user names, passwords and seeds in
`browser_run` stdout/stderr and `browser_file` text before truncating or storing
the output, summaries or server trail. The replacement is `[REDACTED]`, without
a secret name. The browser box is not trusted and receives no credential list.
Its log no longer includes command previews. A missing key or unreadable Login
fails closed with a fixed error instead of returning browser output.

Nonempty values of four or more characters match anywhere. Values of one to
three characters match only whole tokens, bounded by characters other than
Unicode letters, digits or underscore. This avoids blanking ordinary words.
HTTP chunks are assembled before scanning. Stdout and stderr are separate.

For each seed, the server computes RFC 6238 SHA-1 codes for the current step
and two steps either side at redaction time. These exact codes match whole
tokens. Raw seeds use six digits and a 30-second period. Whitespace and hyphens
are removed and letters uppercased for computing codes. An `otpauth://totp`
URI supplies its `secret`, `digits` and `period` (defaults six and 30).
Only SHA-1 is supported. Invalid seed/configuration skips that Login's computed
codes and logs its name, never its value. Its raw seed, user name and password
are still masked. State read errors still fail closed.
Codes older than about two periods (normally 60 seconds) at redaction time are
not masked (expired).

Limits: this is raw exact-value matching, not URL/base64 decoding, OCR or
protection against deliberate encoding, arbitrary splitting or indirect
extraction. Images and downloaded assets are not scanned. Partial values
already truncated by the box cannot be recognized. Model-supplied tool inputs
are not made secret retroactively. The browser-filling risks above still apply.
