import { describe, expect, it } from "vitest";
import type { ClientBuild } from "../api";
import { buildsFor } from "./Apps";

const build = (kind: ClientBuild["kind"], platform: ClientBuild["platform"]): ClientBuild => ({
  platform,
  kind,
  arch: "x86_64",
  version: "0.1.0",
  build: "release",
  size: 1,
  sha256: "0".repeat(64),
  filename: `TheVault_0.1.0.${kind}`,
  published: 0,
  path: `/api/clients/TheVault_0.1.0.${kind}`,
});

describe("buildsFor", () => {
  it("keeps one platform's builds, the AppImage before the packages", () => {
    const all = [build("rpm", "linux"), build("apk", "android"), build("deb", "linux"), build("appimage", "linux")];
    expect(buildsFor(all, "linux").map((b) => b.kind)).toEqual(["appimage", "deb", "rpm"]);
    expect(buildsFor(all, "android").map((b) => b.kind)).toEqual(["apk"]);
    expect(buildsFor([], "android")).toEqual([]);
  });
});
