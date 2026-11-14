#!/usr/bin/env bash
# Place before the heavy wrapper so refused builds never acquire a slot.
set -euo pipefail
root=${TARGET_CLEANUP_ROOT:-/src}
floor=${TARGET_MIN_FREE_GIB:-300}
[[ $floor =~ ^[0-9]+$ && ${#floor} -le 6 ]] || { printf 'Invalid TARGET_MIN_FREE_GIB\n' >&2; exit 1; }
available=$(df -Pk -- "$root" | awk 'END {print $4}')
[[ $available =~ ^[0-9]+$ ]] || exit 1
if (( available < floor * 1024 * 1024 )); then
    printf 'Build refused: %s KiB free, need %s GiB. Run scripts/target-cleanup.sh --dry-run with current card states.\n' "$available" "$floor" >&2
    exit 1
fi
(( $# )) || { printf 'Usage: target-build.sh command [args...]\n' >&2; exit 1; }
exec "$@"
