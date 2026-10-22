# The TheVault server image: vault-server, the `vault` CLI and the built web client, run as uid
# 1000 with the vault in /data and its secret key in /config (OQ-S2). Build it from the repository
# root with `just image-build`, or `just image-clients-build <dir>` to bake in the APK and AppImage
# of this version that `just publish-clients` wrote to <dir>.
#
#   docker build -f docker/Dockerfile --platform linux/amd64 -t thevault .
#   docker build -f docker/Dockerfile --platform linux/amd64 --build-context clients=<dir> -t thevault .
#
# Without a `clients` build context the image serves no app downloads.

# The empty default for the `clients` build context, which --build-context replaces.
FROM scratch AS clients

FROM node:22.20.0-bookworm-slim AS web
RUN npm install -g pnpm@10
WORKDIR /src
# vite.config.ts reads the workspace version from the root Cargo.toml.
COPY Cargo.toml ./
COPY web/package.json web/pnpm-lock.yaml web/.npmrc web/
RUN --mount=type=cache,target=/pnpm-store \
    pnpm -C web install --frozen-lockfile
COPY web web
ARG THEVAULT_WEB_COMMIT=""
RUN THEVAULT_WEB_COMMIT="${THEVAULT_WEB_COMMIT}" pnpm -C web build

FROM rust:1.97-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY crates crates
COPY docs/vault-format docs/vault-format
COPY docs/vault-format.md docs/vault-format.md
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p server -p cli --bins \
 && mkdir -p /out \
 && cp target/release/vault-server target/release/vault /out/ \
 && sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml > /out/VERSION \
 && test -s /out/VERSION

FROM debian:bookworm-slim
# ca-certificates for anything that reads the system roots. The server's own HTTPS (Gmail,
# Calendar, OAuth, scripts' http) uses rustls with bundled webpki roots, so it does not need them.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 thevault \
 && useradd --uid 1000 --gid 1000 --home-dir /home/thevault --create-home --shell /usr/sbin/nologin thevault \
 && mkdir -p /data /config /opt/thevault/clients \
 && chown 1000:1000 /data /config

COPY --from=rust /out/vault-server /out/vault /usr/local/bin/
COPY --from=rust /out/VERSION /opt/thevault/VERSION
COPY --from=web /src/web/dist /opt/thevault/web
# A bind mount rather than COPY --from: BuildKit let a COPY from the empty default stage reuse the
# cached layer of an earlier build that had clients.
RUN --mount=type=bind,from=clients,target=/mnt/clients \
    cp -R /mnt/clients/. /opt/thevault/clients/
COPY docker/entrypoint.sh /usr/local/bin/thevault-entrypoint
COPY docker/healthcheck.sh /usr/local/bin/thevault-healthcheck

ENV VAULT_SECRET_KEY_FILE=/config/vault.key \
    VAULT_CLIENTS_DIR=/opt/thevault/clients \
    XDG_CONFIG_HOME=/config \
    VAULT_DIR=/data \
    HOME=/home/thevault

USER 1000:1000
VOLUME ["/data", "/config"]
EXPOSE 8080
# vault-server shuts down gracefully on SIGINT, ending event streams and waiting on requests.
STOPSIGNAL SIGINT
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --start-interval=1s --retries=3 \
    CMD ["/usr/local/bin/thevault-healthcheck"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thevault-entrypoint"]
