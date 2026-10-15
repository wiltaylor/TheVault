# Development toolchain for TheVault. Every build, test and run command goes
# through this image via ./dev, so the host needs only docker. ./dev tags the
# image with a hash of this file, so editing it rebuilds on the next call.
FROM rust:1.97-bookworm

ARG NODE_VERSION=22.20.0
ARG JUST_VERSION=1.43.0
ARG TAURI_CLI_VERSION=2.12.3
ARG TAURI_DRIVER_VERSION=2.1.0
ARG CARGO_NDK_VERSION=4.1.2
ARG ANDROID_CMDLINE_TOOLS=13114758
ARG ANDROID_PLATFORM=36
ARG ANDROID_BUILD_TOOLS=36.0.0
ARG ANDROID_NDK=27.2.12479018

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq sqlite3 xz-utils unzip file pkg-config libssl-dev \
      build-essential libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
      librsvg2-dev libsecret-1-dev libxdo-dev libgtk-3-dev \
      xvfb xauth webkit2gtk-driver dbus-x11 \
      openjdk-17-jdk-headless \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      | tar -xJ -C /usr/local --strip-components=1 \
 && npm install -g pnpm@10

RUN curl -fsSL "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
      | tar -xz -C /usr/local/bin just

RUN rustup component add rustfmt clippy \
 && rustup target add aarch64-linux-android x86_64-linux-android \
 && chmod -R a+rX /usr/local/rustup

# Installed into /usr/local/cargo/bin, before CARGO_HOME moves to the /cargo
# volume, so the volume does not hide them. cargo-tauri answers to `tauri` too.
RUN cargo install --locked tauri-cli --version "${TAURI_CLI_VERSION}" \
 && cargo install --locked tauri-driver --version "${TAURI_DRIVER_VERSION}" \
 && cargo install --locked cargo-ndk --version "${CARGO_NDK_VERSION}" \
 && ln -s /usr/local/cargo/bin/cargo-tauri /usr/local/bin/tauri \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

ENV JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    NDK_HOME=/opt/android-sdk/ndk/${ANDROID_NDK} \
    ANDROID_NDK_HOME=/opt/android-sdk/ndk/${ANDROID_NDK}
ENV PATH=${PATH}:${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools

RUN mkdir -p "$ANDROID_HOME/cmdline-tools" \
 && curl -fsSL -o /tmp/cmdline-tools.zip \
      "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
 && unzip -q /tmp/cmdline-tools.zip -d "$ANDROID_HOME/cmdline-tools" \
 && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
 && rm /tmp/cmdline-tools.zip \
 && yes | sdkmanager --licenses >/dev/null \
 && sdkmanager --install \
      "platform-tools" \
      "platforms;android-${ANDROID_PLATFORM}" \
      "build-tools;${ANDROID_BUILD_TOOLS}" \
      "ndk;${ANDROID_NDK}" \
 && chmod -R a+rwX "$ANDROID_HOME"

ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright
RUN npx -y playwright@1.56.1 install --with-deps chromium \
 && chmod -R a+rX /opt/playwright

ENV CARGO_HOME=/cargo PNPM_HOME=/pnpm-store GRADLE_USER_HOME=/gradle
RUN mkdir -p /cargo /pnpm-store /gradle && chmod 1777 /cargo /pnpm-store /gradle
