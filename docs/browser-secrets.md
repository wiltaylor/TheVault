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
No origins means any site, with a warning. Browser filling, agent grants and
confirmation cards are separate work, not enabled by this storage feature.

Known limits for browser filling:

- R1: A password must be in the page to work. A misled agent with a shell in
  the browser box could reach it indirectly. This is assumed, not tested.
- R2: Prompt injection can make an agent sign in and act.
- R3: A Login with no origin list can be entered on a look-alike page.

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
