# The Android box: the browser box (docker/browser.Dockerfile) with the Android SDK, adb and an
# x86_64 Android emulator with Google Play. The emulator window runs on the same XFCE desktop and
# noVNC, and agents drive it through the same exec service on port 7700 with `adb`, as they drive
# Chromium with playwright-cli. Build the browser box first, then this on top of it, from the
# repository root with `just android-image-build`, or check it end to end with
# `just android-image-test`:
#
#   docker build -f docker/browser.Dockerfile --platform linux/amd64 -t thevault-browser:local .
#   docker build -f docker/android.Dockerfile --platform linux/amd64 \
#     --build-arg BROWSER_IMAGE=thevault-browser:local -t thevault-android:local .
#
# The build accepts the Android SDK licences, and the Google Play system image may not be
# redistributed, so this image is built locally and never pushed. It needs BROWSER_TOKEN, as the
# browser box does, and /dev/kvm (`--device /dev/kvm`), and refuses to start without either.
ARG BROWSER_IMAGE=thevault-browser:local
FROM ${BROWSER_IMAGE}

ARG ANDROID_CMDLINE_TOOLS=13114758
ARG ANDROID_SYSTEM_IMAGE="system-images;android-35;google_apis_playstore;x86_64"
ARG ANDROID_DEVICE=pixel_7

ENV ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    ANDROID_USER_HOME=/home/agent/.android \
    ANDROID_AVD_HOME=/home/agent/.android/avd \
    ANDROID_SYSTEM_IMAGE=${ANDROID_SYSTEM_IMAGE} \
    ANDROID_DEVICE=${ANDROID_DEVICE}
ENV PATH=${PATH}:${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${ANDROID_HOME}/emulator

USER root

# Java for sdkmanager and avdmanager (bookworm-slim leaves out the man folder its postinst needs),
# and the libraries the emulator loads that Chromium's do not already cover.
RUN mkdir -p /usr/share/man/man1 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      openjdk-17-jre-headless unzip \
      libpulse0 libnss3 libxcomposite1 libxcursor1 libxdamage1 libxi6 libxtst6 libxrandr2 \
      libasound2 libgl1 libegl1 libxkbfile1 libxkbcommon0 libxkbcommon-x11-0 libdrm2 libgbm1 \
      libbsd0 libc++1 libxcb-cursor0 \
 && rm -rf /var/lib/apt/lists/*

# The SDK: platform-tools (adb), the emulator and the Google Play system image. sdkmanager fetches
# for itself, so it gets up to five tries, 15 seconds apart.
RUN mkdir -p "$ANDROID_HOME/cmdline-tools" \
 && curl -fsSL --retry 5 --retry-all-errors --retry-delay 5 -o /tmp/cmdline-tools.zip \
      "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
 && unzip -q /tmp/cmdline-tools.zip -d "$ANDROID_HOME/cmdline-tools" \
 && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
 && rm /tmp/cmdline-tools.zip \
 && yes | sdkmanager --licenses > /dev/null \
 && n=1 \
 && until sdkmanager --install "platform-tools" "emulator" "${ANDROID_SYSTEM_IMAGE}"; do \
      [ "$n" -ge 5 ] && exit 1; n=$((n + 1)); sleep 15; \
    done \
 && chmod -R a+rX "$ANDROID_HOME"

COPY docker/android/profile.sh /etc/profile.d/android.sh
COPY docker/android/emulator.sh /usr/local/bin/thevault-emulator
COPY docker/android/entrypoint.sh /usr/local/bin/thevault-android-entrypoint
COPY docker/android/supervisord.conf /tmp/android-supervisord.conf

# The emulator becomes one more supervisord program. The exec service runs `bash -lc`, whose
# /etc/profile resets PATH, so profile.d puts the SDK back on it. sdkmanager left a root-owned
# ANDROID_USER_HOME behind, which the emulator creates again as agent.
RUN rm -rf /home/agent/.android \
 && cat /tmp/android-supervisord.conf >> /etc/thevault-browser/supervisord.conf \
 && rm /tmp/android-supervisord.conf \
 && su agent -c 'bash -lc "command -v adb emulator avdmanager"' > /dev/null

USER agent
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/thevault-android-entrypoint"]
