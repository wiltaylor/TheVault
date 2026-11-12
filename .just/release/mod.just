# Run on release_target only. The daemon provisions DOCKER_HOST and secrets.
publish *args:
	python3 scripts/release/release.py publish {{args}}

dry-run *args:
	python3 scripts/release/release.py plan {{args}}

# Lead-owned cut gates on the frozen release checkout, never per-card.
gate-dev:
	just check
	python3 scripts/release/web_gate.py

# Build fixture bundle on dev. Transfer it and run device.sh on android as
# documented. A skipped/busy device run does not satisfy this gate.
gate-android:
	just validate-android
	bash scripts/nightly/build.sh emulator

# Run on the dedicated Docker test VM, not the shared release relay.
gate-box:
	bash docker/test-browser-image.sh

# On the Android target call bash directly (it has no just binary).
signed-smoke:
	bash scripts/release/device.sh
