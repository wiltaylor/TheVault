# Git sync between instances

Each publisher stores a monotonic sequence in its authenticated manifest. Receivers keep
an encrypted, durable per-peer high-water checkpoint in state.db, separate from merge bases.
Lower sequences and equal sequences with different referenced publication content are refused with a round
warning before import or archive. Identical retries remain allowed. Rounds hold a vault-local
file lock through publication, and reserve a sequence durably before pushing. Failed pushes
can leave harmless gaps. Unchanged publication keeps its sequence and avoids another push.

Checkpoints fingerprint the manifest blob and the note and definition blobs it references,
not unrelated files in the git tree. Assets are pinned by their id and the assembled SHA256
from the authenticated manifest, not by their part blobs, so a damaged part cannot change a
checkpoint; the asset still fails its own check at import. Each definition includes a
plaintext SHA256, verified before import, as notes and assembled assets are verified.

If the publisher's own remote manifest is missing, cannot be decrypted (for example one
written before sequences existed) or cannot be parsed, the round warns and publishes over it
from the local counter. Only an authenticated manifest that names a different instance
stops publishing.

Upgrade every syncing vault together. Format-2 manifests without a positive sequence or
without a plaintext SHA256 on every definition are
refused with upgrade guidance, never accepted through a legacy fallback. Retained local
vaults republish with sequences on their next round. No remote history migration or deletion
is needed. The first sequenced contact is trust on first use, not proof of freshness before
that contact. A new device without checkpoints or a restored/rolled-back state.db cannot
detect older snapshots it has never recorded. Checkpoints survive disconnect. Do not delete
them to silence a warning. Corrupt checkpoints fail closed and require owner investigation.

After restoring state.db, the publisher reserves above the larger of its local counter and
the authenticated sequence in its own fetched remote manifest. If the remote was also
rolled back, it cannot recover the lost high-water mark. Do not run two copies of the same
vault identity concurrently: their local locks do not coordinate across machines. Give an
independent copy a fresh vault identity before syncing, or keep the original stopped while
restoring its replacement.

The shared symmetric key authenticates the group, not individual publishers. Anyone with
that key can publish for another instance and raise its sequence, even to the maximum,
freezing later legitimate publications. After investigating and removing the untrusted
key holder, an administrator can delete the affected receiver checkpoint via
`DELETE /api/secrets/gitsync_freshness_receive_<id>` (admin scope), then reconnect to a
trusted publication. PUT to this reserved name is refused, so it cannot forge a checkpoint.
This deliberately resets trust on first use and loses rollback
protection for that peer until a new checkpoint is recorded. If the forged maximum is on
the publisher's remote branch too, restore a trusted publication there before resuming.
These recovery steps are not automatic and are not needed for a normal upgrade.

Agent thread transcripts stay on their home server. Git sync neither exports them nor imports
them from peer branches, including older branches that already contain transcripts. Removing
an old exported copy leaves the local thread intact. Device sync still mirrors server threads.

Git sync shares chosen notes between your own TheVault instances through a git repo you already have, such as a private repo on GitHub. Each instance publishes the notes that match its export query, and takes the published notes of the other instances that match its import query. Everything an instance writes to the repo is encrypted with a key that only your instances hold.

Git sync is not device sync. The desktop and Android apps sync with their own home server (see [sync.md](sync.md)). Git sync connects separate home servers, each with a vault of its own, for example one at home and one at work.

This guide covers setting up the repo and each instance, the rules a round follows, and what someone who can read the repo learns from it.

## What you need

- Two or more TheVault servers, each signed in to its web app.
- An empty git repo that every instance can reach. A private GitHub repo works. Create it with no README, so it has no commits.
- On each instance, one way to sign in to the repo:
  - **HTTPS with a token**, for an instance whose network blocks SSH; or
  - **SSH with a key pair**, which TheVault makes for you.

Instances on the same repo can use different ways to sign in.

## How a round works

Each instance runs a round on its schedule, every 15 minutes unless you change it, or when you select **Sync now**. A round:

1. Fetches every instance's branch from the repo.
2. Reads each other instance's published notes, runs this instance's import query over them, and writes the matches into this vault.
3. Archives the imported notes another instance stopped publishing or the import query no longer matches.
4. Encrypts this instance's export set and pushes it to this instance's own branch, as a single commit that replaces the one before it.

An imported note keeps its id, its type and its assets. Its history names the instance it came from as the author, as `gitsync <name>`. When a note uses a type or taxonomy this vault does not have, the round creates it. It never changes a type or taxonomy this vault already has.

The server runs the `git` program for each round. The Docker image includes `git` and `ssh`.

## Set up the first instance

1. Open **Settings > Sync**.
2. Under **Sync key**, select **Generate key**. The page shows the key, which starts with `tvsync1-`, once. Select **Copy sync key** and keep it somewhere safe until every instance has it. After you leave or reload the page, no page or command shows it again.
3. Under **Connection**, enter the repo's address in **Remote URL** and choose how to sign in. Follow [HTTPS with a GitHub token](#https-with-a-github-token) or [SSH with a deploy key](#ssh-with-a-deploy-key).
4. Under **Queries**, write the **Export query** and the **Import query**. See [The queries](#the-queries).
5. Under **Schedule**, check the **Instance name**. Other instances show it in the history of the notes this one sends. It defaults to the vault folder's name.
6. Select **Save settings**. A query or schedule that does not check is refused, and the reason shows under it.
7. Under **Status**, select **Sync now**. The page shows **Syncing…** until the round ends, then its report.

## Set up each other instance

Do the same on each other instance, with one difference at step 2: paste the key from the first instance into **Paste a key** and select **Save key**. Do not generate a new key. Every instance on the repo must hold the same key.

## HTTPS with a GitHub token

Use a fine-grained personal access token that can write to the one repo and nothing else.

1. On GitHub, open **Settings > Developer settings > Personal access tokens > Fine-grained tokens** and select **Generate new token**.
2. Name it, for example `TheVault sync laptop`, and choose an expiry.
3. Under **Repository access**, choose **Only select repositories** and pick the sync repo.
4. Under **Permissions > Repository permissions**, set **Contents** to **Read and write**. Leave every other permission at **No access**. GitHub adds **Metadata: Read-only** on its own.
5. Select **Generate token** and copy it.

Then, in TheVault, on **Settings > Sync**:

1. Set **Remote URL** to the repo's HTTPS address, such as `https://github.com/you/vault-sync.git`.
2. Choose **HTTPS with token** in **Sign in with**.
3. Leave **Username** as `x-access-token`. For another host, enter the username it expects with the token.
4. Paste the token into **Token** and select **Save token**.
5. Select **Save settings**.

The page never shows the token again. The field says `set` while a token is stored. To replace it, paste a new one and select **Save token**. **Remove token** deletes it.

The server hands the token to `git` through a helper script it deletes after the round. The token is never in the remote URL or on a command line, and the server removes it from any error it records.

When the token expires, rounds fail with an authentication error. Make a new token and save it.

## SSH with a deploy key

A deploy key gives one key pair access to one repo.

1. On **Settings > Sync**, set **Remote URL** to the repo's SSH address, such as `git@github.com:you/vault-sync.git`.
2. Choose **SSH key** in **Sign in with**.
3. Select **Make a key pair**. The page shows the public key. Select **Copy public key**.
4. On GitHub, open the repo's **Settings > Deploy keys** and select **Add deploy key**. Paste the public key, name it after the instance, tick **Allow write access**, and select **Add key**.
5. Select **Save settings**, then **Sync now**.

To use a key pair you already have, paste its OpenSSH private key into **Or paste an OpenSSH private key** and select **Save private key**. The page then shows its public key. The page never shows a private key.

TheVault includes GitHub's published host keys. For any other host, the first round over SSH trusts the host's key and pins it, and the page then shows its fingerprint beside **Host key**. Check that fingerprint against the one your host publishes. If the host's key changes later, rounds stop with a host key error rather than connect.

## The queries

Both queries use the query language of the rest of the app (PRD Appendix B).

- The **export query** chooses the notes this instance publishes. It runs over this vault, as any query does.
- The **import query** chooses which of another instance's published notes this instance takes. It runs over that instance's published notes only.

The import query has these limits:

- It cannot use `this`. Saving a query that does is refused.
- Links and `linked_from` see only the notes the other instance published. A note it did not publish does not match.
- `under()` uses the other instance's taxonomy.
- `today()` uses this instance's clock.

A note this instance created and another instance published back, after editing it, is always taken, whatever the import query says.

If a note this instance imported stops matching its import query while the other instance still publishes it, the round archives this instance's copy, as it does for a note that is no longer published. When the note matches again, the round unarchives it. A note you archived yourself stays archived, and a note this instance created is never archived this way. If the import query cannot run against an instance's notes, the round reports a warning and archives none of that instance's notes.

## Edits, clashes and deletes

**Edits flow back.** When you edit an imported note, this instance publishes the fields you changed, and the instance the note came from takes them on its next round.

**The origin wins a clash.** Each note has an origin: the instance that created it. When two instances changed the same field of a note, the origin's value wins. Fields only one side changed are merged. The losing version stays in the note's history, and the instance that lost gets a notification naming the note and the fields that clashed. No conflict copy is made.

**Links to notes you were not sent are dropped.** When an imported note links to a note this instance does not have, the round drops that link and keeps the rest of the note. When your edits flow back to the origin, the origin keeps the link.

**A note that is no longer published is archived.** When another instance stops publishing a note, because it no longer matches the export query or was deleted there, the round archives this instance's copy. The round also archives a copy that no longer matches this instance's import query. When the note is published and matches again, the round unarchives it. A note you archived yourself stays archived.

**A note you delete stays deleted.** When you delete an imported note, later rounds do not bring it back. If you restore it from the trash, it syncs again.

## Size limits

An instance's published branch holds its encrypted notes and assets. **Settings > Sync** shows the size of the last push.

- Past **1 GB**, Settings > Sync and the activity log warn you.
- Past **4 GB**, the push is refused, and you get a notification. The instance still imports.

Change both under **Schedule**, in **Warn above (MB)** and **Refuse above (MB)**. To shrink the branch, narrow the export query.

## When a round fails

**Settings > Sync** shows the last error, and the activity log records it as a `gitsync.round` entry. You get a notification the first time a given error happens, not on every round that repeats it.

| Message | Cause and fix |
|---|---|
| Key mismatch | Another instance's branch was written with a different sync key. Paste the current key into this instance. The round skips that branch and still publishes this instance's own. |
| Update this instance | Another instance runs a newer release, with a newer repo format. Update this instance. The round skips that branch until then. |
| Authentication failed | The token is wrong, expired or lacks **Contents: Read and write**, or the deploy key lacks write access. |
| Host key | The SSH host's key changed since it was pinned. Check with your host before you disconnect and set up again. |
| Size refused | The published branch is over the refuse limit. Narrow the export query, or raise the limit. |

## Format 2 upgrade

Format 2 binds every encrypted object to its publishing instance, object kind
and logical ID, and each asset part to its position. The canonical keyed path
is authenticated and included in deterministic nonce derivation. Notes must
also match the manifest's type and plaintext hash. Unchanged context and
content still produce identical blobs.

Before upgrading, run a last sync on the old build so that every instance has imported the others' latest changes.

There is no legacy decryption fallback. Format 1 branches are skipped with an
upgrade message, not imported or treated as deleted content. Upgrade every
instance, retaining each local vault and the existing sync key, then run sync
on each instance to republish its local data in format 2. An older build cannot
read format 2 blobs. Mixed-version peers do not exchange changes until upgraded.
Do not remove the only local copy of data before that peer has republished it.
This requires no key rotation or manual repository history rewrite.

Context binding does not detect replay of an older valid snapshot at the same
context. Durable peer freshness is separate work (TV-264).

## Rotating the sync key

To rotate the key, select **Generate a new key** on one instance. The next round on that instance re-encrypts its whole branch with the new key. Every other instance reports a key mismatch for that branch until you paste the new key into it.

## What the repo reveals

Every file in the repo is encrypted with the sync key and named with a keyed hash. Someone who can read the repo but has no key sees:

- branches named `tv-` followed by hex, one per instance;
- in each branch, one commit, holding one folder with a hex name;
- in that folder, hex-named files: one per published note, one per type or taxonomy, and each asset in parts of at most 40 MB.

From those, an observer can learn how many instances sync, about how many notes and assets each one publishes, and their sizes. They cannot learn titles, contents, note ids, type names or which notes link to which. Because each branch holds one commit that replaces the last, the repo does not keep old versions of your notes, and it does not grow with every round.

Unchanged notes encrypt to identical files, so an observer who compares two pushes can see how many notes changed.

## What is stored, and where

Every setting and credential is a secret in the server's `state.db`, encrypted with the vault's secret key. None is in a vault file or the repo.

| Secret | What it holds |
|---|---|
| `gitsync_settings` | The remote URL, the sign-in choice and username, both queries, the schedule, the instance name, the size limits and whether the schedule is paused |
| `gitsync_token` | The HTTPS token |
| `gitsync_ssh_key` | The SSH private key |
| `gitsync_known_hosts` | The pinned SSH host keys |
| `gitsync_key` | The sync key |
| `gitsync_state` | The last run, the last error, the last report and the host key fingerprint |

The server keeps its clone of the repo in `.vault/gitsync/`. A backup made with `vault backup` carries the secrets in `state.db`, readable only with the vault's secret key, as before.

## Disconnect

Select **Disconnect** under **Status**. TheVault deletes every `gitsync_*` secret, the clone in `.vault/gitsync/`, and what it remembered about each synced note. Notes already synced stay in the vault. The instance's branch stays in the repo; delete it on the host if you want it gone. To stop the token or deploy key from working, revoke it on the host too.

## From the command line

`vault gitsync` does everything the page does, with an `admin` token:

| Command | What it does |
|---|---|
| `vault gitsync status` | The settings, which secrets are set, whether a round runs, and the last round |
| `vault gitsync set --remote-url … --auth https --export-query … --import-query …` | Changes the settings given. Other flags are `--https-username`, `--schedule`, `--name`, `--warn-bytes`, `--max-bytes` and `--paused true\|false` |
| `vault gitsync token set` | Stores the token, read from stdin |
| `vault gitsync token clear` | Removes the token |
| `vault gitsync ssh-key generate` | Makes a key pair and prints the public key |
| `vault gitsync ssh-key set` | Stores a private key read from stdin and prints its public key |
| `vault gitsync key generate` | Makes a sync key and prints it, the only time it is shown |
| `vault gitsync key set` | Stores a sync key read from stdin |
| `vault gitsync run` | Runs a round, waits for it, and prints the status with its report. `--no-wait` returns as soon as it starts |
| `vault gitsync disconnect` | Disconnects, as the page's **Disconnect** does |

For example, to pass a key from one instance to another:

```sh
vault --url https://home.example gitsync key generate
printf '%s' 'tvsync1-…' | vault --url https://work.example gitsync key set
```

## API

The routes are under `/api/integrations/gitsync` and need the `admin` scope. `GET /api/integrations/gitsync` answers `{configured, settings, token_set, ssh_key_set, ssh_public_key, host_fingerprint, key_set, running, last_run, last_error, last_report}` and never a secret value. The OpenAPI document lists every route.

## Why the app now touches git

The PRD's decision D24 said the app never touches git. Git sync reverses that for this one purpose, as git-sync D1 records. The vault folder itself is still not a git repo, and note history still lives in `state.db` (PRD D13). Git is only the transport between instances.
