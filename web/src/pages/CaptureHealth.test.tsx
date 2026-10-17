import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Device } from "../api";
import type { CaptureHealthStatus } from "../deviceImport";

const captureHealth = vi.fn();
const openCaptureFix = vi.fn();
vi.mock("../deviceImport", async (orig) => {
  const real = await orig<typeof import("../deviceImport")>();
  return { ...real, thisPhone: { ...real.thisPhone, captureHealth, openCaptureFix } };
});

const devices = vi.fn();
const deviceFiles = vi.fn();
const clients = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, devices, deviceFiles, clients } };
});
vi.mock("../inApp", async (orig) => {
  const real = await orig<typeof import("../inApp")>();
  return { ...real, appInfo: () => Promise.resolve(null) };
});

const { CaptureHealth, healthWarning, stateLabel } = await import("./CaptureHealth");
const { Devices } = await import("./Devices");

const sick: CaptureHealthStatus = {
  paired: true,
  setup_done: true,
  kinds: [
    { kind: "sms", label: "SMS", state: "permission_missing", message: "Permission missing: Android has not given TheVault RECEIVE_SMS, READ_SMS", fix: "app_details" },
    { kind: "calls", label: "Calls", state: "working", message: "Working", fix: null },
    {
      kind: "rcs",
      label: "RCS",
      state: "restricted",
      message: "Blocked by restricted settings",
      fix: "app_details",
      then: "notification_access",
    },
    { kind: "contacts", label: "Contacts", state: "not_set_up", message: "Not set up", fix: "capture_setup" },
  ],
  battery: {
    optimised: true,
    samsung: true,
    message: "Battery optimisation is on",
    fix: "battery",
    samsung_line: "On a Samsung phone, also keep TheVault out of Sleeping apps",
  },
  background: [
    { work: "catch_up", label: "Catch-up of messages, calls and contacts", last: null, expected: true, stale: true },
    { work: "commands", label: "Device command check", last: null, expected: false, stale: false },
  ],
  problems: ["SMS: permission missing", "Battery optimisation is on"],
  checked: 1_760_000_000_000,
};

const phone: Device = {
  id: "01J9Z3K8Q2W6E4R5T7Y8U9I0OP",
  name: "Galaxy",
  platform: "android",
  capabilities: [],
  token: "device-x",
  push: true,
  last_seen: 1_760_000_000_000,
  created: 1_750_000_000_000,
  settings: { commands: [], contacts_write: false, sms_send: false, exclusions: [], skip_pattern: "" },
  health: { problems: ["SMS: permission missing", "Battery optimisation is on"], reported: 1_760_000_000_000 },
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("CaptureHealth", () => {
  it("lists each capture kind's state with a button that opens the system screen that fixes it", async () => {
    captureHealth.mockResolvedValue(sick);
    openCaptureFix.mockResolvedValue(undefined);
    const m = render(() => <CaptureHealth />);
    await waitFor(() => expect(m.container.querySelector('[data-kind="sms"]')).not.toBeNull());
    const item = (kind: string) => m.container.querySelector(`[data-kind="${kind}"]`) as HTMLElement;

    expect(item("sms")).toHaveAttribute("data-state", "permission_missing");
    expect(item("sms").textContent).toContain("Permission missing");
    expect(item("calls").textContent).toContain("Working");
    expect(item("calls").querySelector("button")).toBeNull();
    expect(item("rcs").textContent).toContain("Blocked by restricted settings");
    expect(item("contacts").textContent).toContain("Not set up");
    expect(m.getByRole("alert").textContent).toContain("Something stops capture");

    fireEvent.click(item("sms").querySelector('[data-fix="app_details"]')!);
    expect(openCaptureFix).toHaveBeenCalledWith("app_details");
    fireEvent.click(item("rcs").querySelector('[data-fix="notification_access"]')!);
    expect(openCaptureFix).toHaveBeenCalledWith("notification_access");
    fireEvent.click(item("contacts").querySelector('[data-fix="capture_setup"]')!);
    expect(openCaptureFix).toHaveBeenCalledWith("capture_setup");

    // Battery: optimised, with the exemption button and the Samsung line.
    expect(item("battery")).toHaveAttribute("data-state", "optimised");
    expect(item("battery").textContent).toContain("Sleeping apps");
    fireEvent.click(item("battery").querySelector('[data-fix="battery"]')!);
    expect(openCaptureFix).toHaveBeenCalledWith("battery");

    // Background work that should run but has not for two hours; work not expected is left out.
    expect(item("catch_up")).toHaveAttribute("data-state", "stale");
    expect(item("catch_up").textContent).toContain("Stalled");
    expect(item("catch_up").textContent).toContain("never");
    expect(item("commands")).toBeNull();
  });

  it("checks again when the app comes back from a system screen", async () => {
    captureHealth.mockResolvedValueOnce(sick).mockResolvedValueOnce({ ...sick, problems: [], kinds: [], battery: { ...sick.battery, optimised: false, fix: null }, background: [] });
    const m = render(() => <CaptureHealth />);
    await waitFor(() => expect(m.container.querySelector('[data-kind="sms"]')).not.toBeNull());
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(m.container.textContent).toContain("Every kind of capture can work."));
    expect(captureHealth).toHaveBeenCalledTimes(2);
  });

  it("says why when the phone cannot answer", async () => {
    captureHealth.mockRejectedValue(new Error("only the Android app can do this"));
    const m = render(() => <CaptureHealth />);
    await waitFor(() => expect(m.getByRole("alert").textContent).toContain("only the Android app"));
  });

  it("names every state", () => {
    expect(stateLabel("working")).toBe("Working");
    expect(stateLabel("permission_missing")).toBe("Permission missing");
    expect(stateLabel("restricted")).toBe("Blocked by restricted settings");
    expect(stateLabel("not_set_up")).toBe("Not set up");
  });
});

describe("the device card", () => {
  it("shows the warning the phone reported, and none for a device with nothing wrong", async () => {
    const quiet: Device = { ...phone, id: "01J9Z3K8Q2W6E4R5T7Y8U9I0OQ", name: "Laptop", platform: "linux", health: null };
    const fixed: Device = { ...phone, id: "01J9Z3K8Q2W6E4R5T7Y8U9I0OR", name: "Pixel", health: { problems: [], reported: 1 } };
    devices.mockResolvedValue([phone, quiet, fixed]);
    deviceFiles.mockResolvedValue([]);
    clients.mockResolvedValue({ clients: [] });
    const m = render(() => <Devices />);
    await waitFor(() => expect(m.container.querySelectorAll(".device-card")).toHaveLength(3));
    const card = (id: string) => m.container.querySelector(`[data-device="${id}"]`) as HTMLElement;
    const warning = card(phone.id).querySelector(".device-health-warning");
    expect(warning).not.toBeNull();
    expect(warning!.textContent).toContain("The phone reports a capture problem: SMS: permission missing");
    expect(warning!.textContent).toContain("Battery optimisation is on");
    expect(warning!.textContent).toContain("Reported");
    expect(card(quiet.id).querySelector(".device-health-warning")).toBeNull();
    expect(card(fixed.id).querySelector(".device-health-warning")).toBeNull();
  });

  it("words the warning from the reported problems", () => {
    expect(healthWarning(phone)).toBe("The phone reports a capture problem: SMS: permission missing; Battery optimisation is on.");
    expect(healthWarning({ health: null })).toBeNull();
    expect(healthWarning({})).toBeNull();
  });
});
