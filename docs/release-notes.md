# Release notes

## Unreleased

- Agent code run with `run_script` or `run_saved_script` now reaches public HTTP addresses only. Workflows that drove LAN services must use a user script tool or task naming the host, or enable `ai_settings.fetch_private`. User-file grants are unchanged. Guarded agent HTTP no longer uses environment proxies.
