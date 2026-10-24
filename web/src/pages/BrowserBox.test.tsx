import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import * as inApp from "../inApp";
import { BrowserBox } from "./BrowserBox";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const running = { configured: true, reachable: true, version: "0.1.12", playwright_cli: "0.1.22", display: ":1", error: null };

describe("the browser box page", () => {
  it("frames the desktop through the server when the box answers", async () => {
    vi.spyOn(inApp, "appInfo").mockResolvedValue(null);
    vi.spyOn(api, "browserStatus").mockResolvedValue(running);
    render(() => <BrowserBox />);
    const frame = (await screen.findByTitle("Browser box desktop")) as HTMLIFrameElement;
    expect(frame.getAttribute("src")).toBe("/browser/vnc.html?autoconnect=1&resize=remote&path=browser/websockify");
    expect(screen.getByText(/playwright-cli 0.1.22/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Open in a new tab" })).toHaveAttribute("target", "_blank");
  });

  it("says so when the box is not set up or does not answer", async () => {
    vi.spyOn(inApp, "appInfo").mockResolvedValue(null);
    const status = vi.spyOn(api, "browserStatus").mockResolvedValue({ ...running, configured: false, reachable: false });
    const r = render(() => <BrowserBox />);
    expect(await screen.findByText(/Not set up/)).toBeTruthy();
    expect(screen.queryByTitle("Browser box desktop")).toBeNull();
    r.unmount();
    status.mockResolvedValue({ ...running, reachable: false, error: "connection refused" });
    render(() => <BrowserBox />);
    expect(await screen.findByRole("alert")).toHaveTextContent("connection refused");
  });

  it("does not frame the desktop in the apps", async () => {
    vi.spyOn(inApp, "appInfo").mockResolvedValue({ platform: "android" } as Awaited<ReturnType<typeof inApp.appInfo>>);
    vi.spyOn(api, "browserStatus").mockResolvedValue(running);
    render(() => <BrowserBox />);
    await waitFor(() => expect(screen.getByText(/opens in the web app/)).toBeTruthy());
    expect(screen.queryByTitle("Browser box desktop")).toBeNull();
  });
});
