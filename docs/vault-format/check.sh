#!/usr/bin/env bash
# Check every example vault file in docs/vault-format/vault against its schema
# with `wcl check`, and against the data-only rule with data-only.mjs. Then
# confirm both checks still reject the files in invalid/.
#
# A type file follows the readers' rule for attributes from newer releases:
# an attribute its schema does not declare is a warning, not a failure
# (tolerated.mjs), so tolerated/newer-type.wcl passes with warnings.
#
# Usage, from the repository root:
#   ./dev docs/vault-format/check.sh
#
# WCL is the wcl binary to run (default: wcl on PATH). Vault files carry no
# import, so each check writes the schema files followed by the example into
# one temporary file and runs `wcl check` on it. Do not pipe into `wcl check -`
# instead: at the pinned WCL revision, stdin input skips schema validation and
# prints OK for any document that parses.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
wcl=${WCL:-wcl}
schema=$here/schema
vault=$here/vault
failed=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

check() {
  local file=$1
  shift
  local rel=${file#"$here"/}
  local combined=$tmp/$(basename "$file")
  cat "$@" "$file" >"$combined"
  if "$wcl" check "$combined" >/dev/null 2>&1; then
    echo "ok    wcl check   $rel"
  else
    echo "FAIL  wcl check   $rel (schema files: ${*##*/})"
    "$wcl" check "$combined" || true
    failed=1
  fi
}

# A type file: an attribute only a newer release knows is a warning.
check_type() {
  local file=$1
  local rel=${file#"$here"/}
  local combined=$tmp/$(basename "$file")
  cat "$schema/common.wcl" "$schema/type.wcl" "$file" >"$combined"
  local json out
  # `wcl check` exits non-zero on any violation; tolerated.mjs decides.
  json=$("$wcl" check --json "$combined" 2>/dev/null || true)
  if out=$(printf '%s' "$json" | node "$here/tolerated.mjs"); then
    if [ -n "$out" ]; then
      echo "ok    wcl check   $rel (with warnings)"
      printf '%s\n' "$out" | sed 's/^/        /'
    else
      echo "ok    wcl check   $rel"
    fi
    return 0
  fi
  echo "FAIL  wcl check   $rel (schema files: common.wcl type.wcl)"
  printf '%s\n' "$out" | sed 's/^/        /'
  return 1
}

for f in "$vault"/types/*.wcl "$here"/tolerated/*.wcl; do
  check_type "$f" || failed=1
done
for f in "$vault"/taxonomies/*.wcl; do
  check "$f" "$schema/taxonomy.wcl"
done
for f in "$vault"/tasks/*.wcl; do
  check "$f" "$schema/task.wcl"
done
for f in "$vault"/agents/*.wcl; do
  check "$f" "$schema/agent.wcl"
done
for f in "$vault"/devices/*.wcl; do
  check "$f" "$schema/device.wcl"
done
for dir in "$vault"/notes/*/; do
  type=$(basename "$dir")
  for f in "$dir"*.wcl; do
    check "$f" "$schema/common.wcl" "$schema/notes/$type.wcl"
  done
done

if ! (cd "$here" && node data-only.mjs $(find vault -name '*.wcl' | sort)); then
  failed=1
fi

# The checks must still reject a bad file. Both of these are specimen notes.
cat "$schema/common.wcl" "$schema/notes/specimen.wcl" "$here/invalid/schema-violation.wcl" \
  >"$tmp/schema-violation.wcl"
if "$wcl" check "$tmp/schema-violation.wcl" >/dev/null 2>&1; then
  echo "FAIL  wcl check   invalid/schema-violation.wcl was accepted"
  failed=1
else
  echo "ok    wcl check   invalid/schema-violation.wcl rejected"
fi
if check_type "$here/invalid/type-wrong-form.wcl" >/dev/null; then
  echo "FAIL  wcl check   invalid/type-wrong-form.wcl was accepted"
  failed=1
else
  echo "ok    wcl check   invalid/type-wrong-form.wcl rejected"
fi
if node "$here/data-only.mjs" "$here/invalid/expression.wcl" >/dev/null; then
  echo "FAIL  data-only   invalid/expression.wcl was accepted"
  failed=1
else
  echo "ok    data-only   invalid/expression.wcl rejected"
fi

exit $failed
