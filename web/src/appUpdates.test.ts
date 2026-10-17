import { describe, expect, it } from "vitest";
import { androidState, desktopState } from "./appUpdates";
import { platformLabel } from "./pages/About";
import { WEB_VERSION, webBuild } from "./version";

describe("desktopState", () => {
  it("says an update is ready to restart into", () => {
    const s = desktopState({
      update: { version: "0.1.4", ready: true },
      updater: { checks: 1, last_checked: 5, available: true },
    });
    expect(s.summary).toBe("0.1.4 ready — restart to update");
    expect(s.restart).toBe(true);
    expect(s.download).toBe(false);
    expect(s.checked).toBe(5);
  });

  it("offers a download when the update is not downloaded", () => {
    const s = desktopState({ update: { version: "0.2.0", ready: false }, updater: { checks: 1 } });
    expect(s.summary).toBe("0.2.0 available");
    expect(s.download).toBe(true);
  });

  it("is up to date after a check that found nothing", () => {
    expect(desktopState({ update: null, updater: { checks: 2, available: true } }).summary).toBe("Up to date");
    expect(desktopState({ update: null, updater: { checks: 0, available: true } }).summary).toBe("Not checked yet");
    expect(desktopState({ update: null, updater: { checks: 1, error: "down" } }).summary).toBe(
      "The last check failed: down",
    );
    const off = desktopState({ update: null, updater: { checks: 0, available: false } });
    expect(off.canCheck).toBe(false);
    expect(desktopState({ settings: { update_check: false } }).automaticOff).toBe(true);
  });
});

describe("androidState", () => {
  it("reads AppUpdateWorker's last status", () => {
    expect(androidState({ status: null, message: null, checked: null }).summary).toBe("Not checked yet");
    expect(androidState({ status: "up_to_date", message: "TheVault 0.1.3 is up to date", checked: 9 })).toMatchObject({
      summary: "Up to date",
      checked: 9,
    });
    expect(androidState({ status: "failed", message: "Cannot reach the server", checked: 9 }).summary).toBe(
      "Cannot reach the server",
    );
  });
});

describe("the About section's labels", () => {
  it("names each platform", () => {
    expect(platformLabel("linux-appimage")).toBe("Linux AppImage");
    expect(platformLabel("linux-package")).toBe("Linux deb/rpm package");
    expect(platformLabel("android")).toBe("Android");
    expect(platformLabel(null)).toBe("Unknown");
  });

  it("bakes the workspace version into the web build", () => {
    expect(WEB_VERSION).toMatch(/^\d+\.\d+\.\d+/);
    expect(webBuild().startsWith(WEB_VERSION)).toBe(true);
  });
});
