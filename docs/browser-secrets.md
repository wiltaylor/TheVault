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
