# Recipes shared by the top-level justfile and its modules.

[private]
require-tool tool:
	@command -v {{tool}} > /dev/null 2>&1 || (echo "{{tool}} is required; run this through ./dev" >&2 && exit 1)

# Install the web client's dependencies from the lockfile
[group('web')]
web-deps-install: (require-tool "pnpm")
	pnpm -C web install --frozen-lockfile

[private]
require-path path ticket:
	@test -e {{path}} || (echo "{{path}} does not exist yet; it lands with {{ticket}}" >&2 && exit 1)

[private]
require-host-tool tool:
	@command -v {{tool}} > /dev/null 2>&1 || (echo "{{tool}} is required; run this on the host, not through ./dev" >&2 && exit 1)
