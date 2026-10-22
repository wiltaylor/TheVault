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

# Every download retries (ticket 138): two release runs failed on dl.google.com, one on a corrupt
# system image zip and one on a curl HTTP/2 stream error. apt retries each fetch, curl downloads
# to a file with --retry before anything unpacks it, and `retry` reruns a command that fetches
# for itself (sdkmanager, the Playwright install) up to five times, 15 seconds apart.
RUN echo 'Acquire::Retries "5";' > /etc/apt/apt.conf.d/80-retries \
 && printf '%s\n' '#!/bin/sh' \
      'n=1' \
      'until "$@"; do' \
      '  [ "$n" -ge 5 ] && { echo "retry: $* failed $n times" >&2; exit 1; }' \
      '  echo "retry: $* failed (attempt $n), trying again in 15s" >&2' \
      '  n=$((n + 1)); sleep 15' \
      'done' > /usr/local/bin/retry \
 && chmod a+rx /usr/local/bin/retry
ARG CURL_RETRY="--retry 5 --retry-all-errors --retry-delay 5"

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq sqlite3 xz-utils unzip file pkg-config libssl-dev \
      build-essential libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
      librsvg2-dev libsecret-1-dev libxdo-dev libgtk-3-dev \
      xvfb xauth webkit2gtk-driver dbus-x11 \
      openjdk-17-jdk-headless \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL $CURL_RETRY -o /tmp/node.tar.xz \
      "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
 && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
 && rm /tmp/node.tar.xz \
 && retry npm install -g pnpm@10

RUN curl -fsSL $CURL_RETRY -o /tmp/just.tar.gz \
      "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
 && tar -xzf /tmp/just.tar.gz -C /usr/local/bin just \
 && rm /tmp/just.tar.gz

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
 && curl -fsSL $CURL_RETRY -o /tmp/cmdline-tools.zip \
      "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
 && unzip -q /tmp/cmdline-tools.zip -d "$ANDROID_HOME/cmdline-tools" \
 && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
 && rm /tmp/cmdline-tools.zip \
 && yes | sdkmanager --licenses >/dev/null \
 && retry sdkmanager --install \
      "platform-tools" \
      "platforms;android-${ANDROID_PLATFORM}" \
      "build-tools;${ANDROID_BUILD_TOOLS}" \
      "ndk;${ANDROID_NDK}" \
 && chmod -R a+rwX "$ANDROID_HOME"

ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright
RUN retry npx -y playwright@1.56.1 install --with-deps chromium \
 && chmod -R a+rX /opt/playwright

ENV CARGO_HOME=/cargo PNPM_HOME=/pnpm-store GRADLE_USER_HOME=/gradle
RUN mkdir -p /cargo /pnpm-store /gradle && chmod 1777 /cargo /pnpm-store /gradle

# The Secret Service the desktop app keeps its device token in, for its S6 tests, and nss_wrapper,
# which gives ./dev's host uid the passwd entry the session bus needs. A layer of its own, so
# adding it rebuilds nothing above.
RUN apt-get update \
 && apt-get install -y --no-install-recommends gnome-keyring libnss-wrapper \
 && rm -rf /var/lib/apt/lists/*

# makepkg, which checks the AUR PKGBUILD in packaging/aur and writes its .SRCINFO. A layer of its
# own, so adding it rebuilds nothing above.
RUN apt-get update \
 && apt-get install -y --no-install-recommends makepkg pacman-package-manager libarchive-tools \
 && rm -rf /var/lib/apt/lists/*

# The Android emulator and an x86_64 Android 15 (API 35) system image, for the manual checks run
# in an emulator (`just android-emulator-checks`), plus the libraries the headless emulator loads.
# ./dev passes /dev/kvm through when the host has it. A layer of its own, so adding it rebuilds
# nothing above.
ARG ANDROID_EMULATOR_IMAGE="system-images;android-35;google_apis;x86_64"
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libpulse0 libnss3 libxcomposite1 libxcursor1 libxdamage1 libxi6 libxtst6 libxrandr2 \
      libasound2 libgl1 libegl1 libxkbfile1 libxkbcommon0 libdrm2 libgbm1 libbsd0 libc++1 \
      qrencode netcat-openbsd \
 && rm -rf /var/lib/apt/lists/* \
 && retry sdkmanager --install "emulator" "${ANDROID_EMULATOR_IMAGE}" \
 && chmod -R a+rwX "$ANDROID_HOME"
ENV PATH=${PATH}:${ANDROID_HOME}/emulator ANDROID_AVD_HOME=/avd
# The AVD lives in the thevault-avd volume ./dev mounts here, so it survives between containers.
RUN mkdir -p /avd && chmod 1777 /avd

# Static Linux builds of the vault CLI (`just cli-release`, ticket 130): the two musl targets,
# zig as the C cross compiler and linker cargo-zigbuild drives (the CLI compiles SQLite, ring and
# zstd), and qemu-user to run the aarch64 build here. Zig comes from its PyPI wheel, a zip of the
# release tarball, since PyPI's CDN is far more dependable than ziglang.org. A layer of its own,
# so adding it rebuilds nothing above.
ARG ZIG_WHEEL=https://files.pythonhosted.org/packages/0c/b7/ecfdce6368cc3664d301f7f52db4fe1004aa7da7a12c4a9bf1de534ff6ab/ziglang-0.13.0-py3-none-manylinux_2_12_x86_64.manylinux2010_x86_64.musllinux_1_1_x86_64.whl
ARG ZIG_WHEEL_SHA256=3ce0c9f16547e5d61b32e0d226926e9a2552ef4b91fccf7ab5ea1a623a77824b
ARG CARGO_ZIGBUILD_VERSION=0.23.4
RUN apt-get update \
 && apt-get install -y --no-install-recommends qemu-user \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL $CURL_RETRY -o /tmp/zig.whl "${ZIG_WHEEL}" \
 && echo "${ZIG_WHEEL_SHA256}  /tmp/zig.whl" | sha256sum -c - \
 && unzip -q /tmp/zig.whl 'ziglang/*' -d /opt \
 && rm /tmp/zig.whl \
 && chmod -R a+rX /opt/ziglang && chmod a+x /opt/ziglang/zig \
 && ln -s /opt/ziglang/zig /usr/local/bin/zig \
 && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
 && chmod -R a+rX /usr/local/rustup \
 && CARGO_HOME=/usr/local/cargo cargo install --locked cargo-zigbuild --version "${CARGO_ZIGBUILD_VERSION}" \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

# Pre-release Linux packages (ticket 139): `just desktop-release` rebuilds the rpm with rpmbuild
# under a version rpm sorts before the release, 0.1.13~alpha.1 for 0.1.13-alpha.1. A layer of
# its own, so adding it rebuilds nothing above.
RUN apt-get update \
 && apt-get install -y --no-install-recommends rpm \
 && rm -rf /var/lib/apt/lists/*
