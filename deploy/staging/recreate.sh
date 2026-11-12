#!/bin/sh
# Deletes only this fixed Compose project's disposable synthetic state.
set -eu
cd "$(dirname "$0")"
unset COMPOSE_FILE COMPOSE_PROJECT_NAME COMPOSE_PROFILES
docker compose -p tv-145-staging -f compose.yaml down --volumes
docker compose -p tv-145-staging -f compose.yaml build
docker compose -p tv-145-staging -f compose.yaml up -d --wait --wait-timeout 180
docker compose -p tv-145-staging -f compose.yaml exec -T staging python3 /app/staging.py smoke
docker compose -p tv-145-staging -f compose.yaml exec -T staging python3 /app/staging.py health
