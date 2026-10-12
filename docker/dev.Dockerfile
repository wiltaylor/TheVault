# Development toolchain for TheVault. Every build, test and run command goes
# through this image via ./dev, so the host needs only docker.
FROM rust:1.97-bookworm

ARG NODE_VERSION=22.20.0
ARG JUST_VERSION=1.43.0

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq sqlite3 xz-utils pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      | tar -xJ -C /usr/local --strip-components=1 \
 && npm install -g pnpm@10

RUN curl -fsSL "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
      | tar -xz -C /usr/local/bin just

RUN rustup component add rustfmt clippy

ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright
RUN npx -y playwright@1.56.1 install --with-deps chromium \
 && chmod -R a+rX /opt/playwright

ENV CARGO_HOME=/cargo PNPM_HOME=/pnpm-store
RUN mkdir -p /cargo /pnpm-store && chmod 1777 /cargo /pnpm-store
