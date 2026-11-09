#!/usr/bin/env python3
"""Prepare a verified local wrapper ZIP for CI, without caching Gradle user data."""
import hashlib
import pathlib
import re
import sys
import time
import urllib.request


def download(url, destination):
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                with destination.open("wb") as output:
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
            return
        except (OSError, ValueError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def prepare(properties, cache):
    text = properties.read_text()
    match = re.search(r"^distributionUrl=https\\://services.gradle.org/distributions/(gradle-([0-9.]+)-bin.zip)$", text, re.M)
    if not match:
        raise ValueError("expected an official Gradle binary distribution URL")
    filename, version = match.groups()
    primary = f"https://services.gradle.org/distributions/{filename}"
    fallback = f"https://github.com/gradle/gradle-distributions/releases/download/v{version}/{filename}"
    cache.mkdir(parents=True, exist_ok=True)
    checksum_file = cache / "checksum.tmp"
    partial = cache / "download.tmp"
    archive = cache / filename
    try:
        download(primary + ".sha256", checksum_file)
        checksum = checksum_file.read_text().strip()
        if not re.fullmatch(r"[0-9a-f]{64}", checksum):
            raise ValueError("invalid official SHA-256")

        def verified(path):
            if not path.is_file():
                return False
            with path.open("rb") as source:
                return hashlib.file_digest(source, "sha256").hexdigest() == checksum

        if not verified(archive):
            try:
                download(primary, partial)
            except (OSError, ValueError):
                print(f"primary download failed, using {fallback}", file=sys.stderr)
                download(fallback, partial)
            if not verified(partial):
                raise ValueError("Gradle ZIP SHA-256 mismatch")
            partial.replace(archive)
        local_url = archive.resolve().as_uri().replace(":", "\\:")
        text = re.sub(r"^distributionUrl=.*$", lambda _: f"distributionUrl={local_url}", text, flags=re.M)
        text = re.sub(r"^distributionSha256Sum=.*\n?", "", text, flags=re.M)
        properties.write_text(text + f"\ndistributionSha256Sum={checksum}\n")
    finally:
        checksum_file.unlink(missing_ok=True)
        partial.unlink(missing_ok=True)


if __name__ == "__main__":
    prepare(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
