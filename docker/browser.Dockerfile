# The browser box (browser B1): an XFCE desktop on TigerVNC display :1, noVNC on port 6080,
# playwright-cli with Chromium, and the exec service browser-agent on port 7700 (browser B2), all
# run as the non-root user agent (uid 1000, home /home/agent, working directory /work) under
# supervisord. It holds no vault data or secrets. Build it from the repository root with
# `just browser-image-build`, or check it end to end with `just browser-image-test`:
#
#   docker build -f docker/browser.Dockerfile --platform linux/amd64 -t thevault-browser .
#
# It needs BROWSER_TOKEN, the bearer token the exec service takes, and refuses to start without.

# The exec service, linked statically against musl like the vault CLI.
FROM rust:1.99-bookworm AS rust
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends musl-tools \
 && rm -rf /var/lib/apt/lists/* \
 && case "${TARGETARCH:-amd64}" in \
      amd64) echo x86_64-unknown-linux-musl ;; \
      arm64) echo aarch64-unknown-linux-musl ;; \
      *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac > /rust-target \
 && rustup target add "$(cat /rust-target)"
WORKDIR /src
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    target=$(cat /rust-target) \
 && cargo build --profile cli --locked -p browser-agent --bin browser-agent --target "$target" \
 && mkdir -p /out \
 && cp "target/$target/cli/browser-agent" /out/ \
 && { ldd /out/browser-agent 2>&1 || true; } | grep -qE 'not a dynamic executable|statically linked'

FROM node:22.20.0-bookworm-slim

# The pinned upstreams: @playwright/cli from npm, and the JetBrains Mono Nerd Font (the
# no-ligature "NerdFontMono" build the web app bundles too) from the ryanoasis/nerd-fonts release.
ARG PLAYWRIGHT_CLI_VERSION=0.1.22
ARG NERD_FONTS_VERSION=v3.5.1
ARG NERD_FONTS_SHA256=04d5e8f903693f9dd13e16f867e994834e681eb3c72c0d337a770dcda09010cf

ENV DEBIAN_FRONTEND=noninteractive \
    PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright \
    PLAYWRIGHT_CLI_VERSION=${PLAYWRIGHT_CLI_VERSION} \
    PLAYWRIGHT_MCP_BROWSER=chromium \
    NO_UPDATE_NOTIFIER=1 \
    DISPLAY=:1 \
    LANG=C.UTF-8

# The desktop, VNC and noVNC, the supervisor, and fonts for common scripts (Playwright's
# dependencies below add CJK, Thai and emoji).
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl xz-utils tini supervisor procps \
      xfce4 xfce4-terminal dbus dbus-x11 x11-xserver-utils x11-utils \
      tigervnc-standalone-server novnc python3-websockify \
      fontconfig fonts-dejavu-core fonts-noto-core \
 && rm -rf /var/lib/apt/lists/*

# JetBrainsMonoNL Nerd Font Mono, the terminal's and the desktop's monospace font.
RUN curl -fsSL --retry 5 -o /tmp/JetBrainsMono.tar.xz \
      "https://github.com/ryanoasis/nerd-fonts/releases/download/${NERD_FONTS_VERSION}/JetBrainsMono.tar.xz" \
 && echo "${NERD_FONTS_SHA256}  /tmp/JetBrainsMono.tar.xz" | sha256sum -c - \
 && mkdir -p /usr/share/fonts/truetype/jetbrains-mono-nerd \
 && tar -xJf /tmp/JetBrainsMono.tar.xz -C /usr/share/fonts/truetype/jetbrains-mono-nerd \
      JetBrainsMonoNLNerdFontMono-Regular.ttf JetBrainsMonoNLNerdFontMono-Bold.ttf \
      JetBrainsMonoNLNerdFontMono-Italic.ttf JetBrainsMonoNLNerdFontMono-BoldItalic.ttf \
      OFL.txt \
 && rm /tmp/JetBrainsMono.tar.xz

# playwright-cli, and Chromium with its system libraries and fonts. playwright-cli runs Chromium
# (as Chrome for Testing, PLAYWRIGHT_MCP_BROWSER=chromium) headless too, so the separate headless
# shell is left out.
RUN npm install -g "@playwright/cli@${PLAYWRIGHT_CLI_VERSION}" \
 && npm cache clean --force \
 && playwright-cli install-browser chromium --with-deps --no-shell \
 && rm -rf /var/lib/apt/lists/* \
 && chmod -R a+rX "${PLAYWRIGHT_BROWSERS_PATH}"

COPY docker/browser/fonts.conf /etc/fonts/local.conf
COPY docker/browser/xfce4-terminal.xml /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-terminal.xml
COPY docker/browser/terminalrc /etc/xdg/xfce4/terminal/terminalrc
COPY docker/browser/supervisord.conf /etc/thevault-browser/supervisord.conf
COPY docker/browser/desktop.sh /usr/local/bin/thevault-desktop
COPY docker/browser/entrypoint.sh /usr/local/bin/thevault-browser-entrypoint
COPY --from=rust /out/browser-agent /usr/local/bin/browser-agent

# The node image's uid 1000 user becomes agent. The panel starts from its default layout rather
# than asking on the first start, the desktop's monospace font is the Nerd Font, and noVNC's
# index is its full client.
RUN usermod --login agent --home /home/agent --move-home --shell /bin/bash node \
 && groupmod --new-name agent node \
 && mkdir -p /work /home/agent/.config /tmp/.X11-unix \
 && chmod 1777 /tmp/.X11-unix \
 && chown -R agent:agent /work /home/agent \
 && cp /etc/xdg/xfce4/panel/default.xml /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml \
 && xsettings=/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xsettings.xml \
 && sed -i 's|<property name="MonospaceFontName" type="[^"]*"[^/]*/>|<property name="MonospaceFontName" type="string" value="JetBrainsMonoNL Nerd Font Mono 10"/>|' "$xsettings" \
 && grep -q 'MonospaceFontName" type="string" value="JetBrainsMonoNL Nerd Font Mono' "$xsettings" \
 && ln -sf vnc.html /usr/share/novnc/index.html \
 && fc-cache -f \
 && fc-match monospace | grep -q 'JetBrainsMonoNL Nerd Font Mono' \
 && su agent -c 'playwright-cli --help' > /dev/null

USER agent
WORKDIR /work
VOLUME ["/home/agent"]
EXPOSE 6080 7700
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --start-interval=1s --retries=3 \
    CMD ["curl", "-fsS", "-o", "/dev/null", "http://127.0.0.1:7700/health"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thevault-browser-entrypoint"]
