#!/usr/bin/env bash
# Input: CARD-ID COLUMN lines. Release deletion requires an explicit basename.
set -euo pipefail
die() { printf '%s\n' "$*" >&2; exit 1; }
root=$(realpath -e -- "${TARGET_CLEANUP_ROOT:-/src}")
[[ $root != / ]] || die 'Refusing filesystem root'
dry=1
cards=/dev/stdin
current=
releases=()
while (($#)); do
    case $1 in
        --apply) dry=0; shift ;;
        --dry-run) dry=1; shift ;;
        --cards) cards=$2; shift 2 ;;
        --current-release) current=$2; shift 2 ;;
        --published-release) releases+=("$2"); shift 2 ;;
        *) die "Unknown argument: $1" ;;
    esac
done
declare -A states=()
while read -r id column extra; do
    [[ -n $id ]] || continue
    [[ $id =~ ^[A-Za-z]+-[0-9]+$ && -n $column && -z $extra ]] || die 'Invalid card input'
    id=${id,,}
    [[ ! -v states[$id] ]] || die "Duplicate card: $id"
    states[$id]=$column
done < "$cards"
paths=()
add() {
    local name=$1 path
    [[ $name != */* && $name != . && $name != .. ]] || die "Refusing path: $name"
    case $name in terrarium*|thevault-heavy*) die "Protected: $name" ;; esac
    path=$root/$name
    [[ -e $path || -L $path ]] || return 0
    [[ ! -L $path && -d $path ]] || die "Refusing non-directory or symlink: $path"
    [[ $(realpath -e -- "$path") == "$root/$name" ]] || die "Outside root: $path"
    # Do not traverse mounted data belonging to another workload.
    ! mountpoint -q -- "$path" || die "Refusing mount: $path"
    paths+=("$path")
}
for id in "${!states[@]}"; do
    case ${states[$id]} in
        done|accepted|archived)
            for suffix in '' -out -next -baseline; do add "$id$suffix"; done ;;
    esac
done
for name in "${releases[@]}"; do
    [[ $name =~ ^rel[A-Za-z0-9._-]+$ ]] || die "Invalid release basename: $name"
    [[ -n $current && $current != */* ]] || die 'Release cleanup requires --current-release'
    [[ $name != "$current" ]] || continue
    add "$name"
done
df -Pk -- "$root"
for path in "${paths[@]}"; do
    printf '%s %s\n' "$([[ $dry == 1 ]] && printf WOULD_REMOVE || printf REMOVE)" "$path"
    if (( ! dry )); then rm -rf --one-file-system -- "$path"; fi
done
df -Pk -- "$root"
