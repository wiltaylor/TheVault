# Recipes shared by the top-level justfile and its modules.

[private]
require-tool tool:
	@command -v {{tool}} > /dev/null 2>&1 || (echo "{{tool}} is required; run this through ./dev" >&2 && exit 1)

# Install the web client's dependencies from the lockfile
[group('web')]
web-deps-install: (require-tool "pnpm")
	pnpm -C web install --frozen-lockfile
