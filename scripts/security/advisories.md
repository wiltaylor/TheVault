# Advisory reporting

Report only. This command never builds, upgrades, signs or rebuilds images.
It does not read vault tokens, proposals or agent rules.

Install Trivy **0.58.2** separately from its verified upstream release.
Supply a separately refreshed Trivy database and saved runtime image archives.
The command refuses another tool version and disables database downloads and
dependency network resolution. Database freshness and archive digest selection
are the schedule owner's responsibility. Archives must cover both server and
browser runtime images, not the development image. An image archive can contain
OS and application dependencies. Filesystem scanning must detect both Cargo.lock
and web/pnpm-lock.yaml. Missing ecosystem results fail the entire report.

```sh
node scripts/security/advisories.mjs . /path/server.tar /path/browser.tar
flock /path/advisories.lock node scripts/security/advisories.mjs --publish . /path/server.tar /path/browser.tar
```

Use an owner-controlled lock shared by all publishers. Publishing looks up a
stable SHA-256 identity label across cards, then creates a backlog item only
when absent. Do not run publishers concurrently without the lock. Existing
cards, including resolved cards, are not reopened. Installed-version changes
produce a new identity. Advisory titles, descriptions, paths, URLs and scanner
stderr are never copied. Only bounded identity fields are emitted. A failed
scanner, invalid report, incomplete coverage or failed backlog write exits 1.
Partial backlog writes remain deduplicated on retry. Reports are not claims
about production or image signatures.

Terrarium integration: the lead should register a project-owned weekly schedule
on thevault-dev, through `/src/thevault-heavy.sh`, with an owned checkout,
external output directory, database refresh policy and digest-identified image
archives. Use `flock` as above and retain stdout JSON and the exit status.
No scanner installation, database refresh or registry access is authorized by
the card's fixture tests. Until these inputs and schedule are installed,
scheduled coverage is **not active** and this slice must not ship as complete.
