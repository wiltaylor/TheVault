import { Route } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GoogleSettings, GoogleSide, GoogleStatus } from "../api";

const googleStatus = vi.fn();
const googleStart = vi.fn();
const googleFinish = vi.fn();
const googleDisconnect = vi.fn();
const runTask = vi.fn();
const googleSaveSettings = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, googleStatus, googleStart, googleFinish, googleDisconnect, googleSaveSettings, runTask },
  };
});

const { GoogleCard, GoogleCallback, redirectMode } = await import("./Integrations");
const M = await import("../workspace/model");
const { PaneRouter } = await import("../workspace/pane");
const { resetWorkspace, workspace } = await import("../workspace/store");

function side(task: string, over: Partial<GoogleSide> = {}): GoogleSide {
  return { task, installed: true, paused: true, last_run: null, last_error: null, ...over };
}

const defaults: GoogleSettings = {
  gmail_mode: "labels",
  gmail_labels: [],
  gmail_window_days: 90,
  gmail_backfill_to: null,
  calendars: ["primary"],
  default_calendar: "primary",
};

const disconnected: GoogleStatus = {
  connected: false,
  client_stored: false,
  account: null,
  settings: defaults,
  gmail: side("google_gmail"),
  calendar: side("google_calendar"),
};

const connected: GoogleStatus = {
  connected: true,
  client_stored: true,
  account: "me@example.com",
  settings: { ...defaults, gmail_labels: ["INBOX"] },
  gmail: side("google_gmail", { paused: false, last_run: "2026-10-18T09:00:00Z" }),
  calendar: side("google_calendar", { paused: false, last_error: "Google answered 500: backend error" }),
};

const AUTH_URL = "https://accounts.google.com/o/oauth2/v2/auth?client_id=abc&state=s1";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("redirectMode", () => {
  it("signs in directly on https and through a pasted loopback address otherwise", () => {
    expect(redirectMode("https:")).toBe("direct");
    expect(redirectMode("http:")).toBe("loopback");
  });
});

describe("GoogleCard", () => {
  it("connects through the loopback paste flow on http", async () => {
    googleStatus.mockResolvedValue(disconnected);
    googleStart.mockResolvedValue({ auth_url: AUTH_URL });
    googleFinish.mockResolvedValue(connected);
    const assign = vi.fn();
    render(() => <GoogleCard protocol="http:" assign={assign} />);

    const link = await screen.findByRole("link", { name: /set up a Google OAuth client/ });
    expect(link.getAttribute("href")).toContain("docs/google.md");
    fireEvent.input(screen.getByLabelText("Client ID"), { target: { value: " abc.apps.googleusercontent.com " } });
    fireEvent.input(screen.getByLabelText("Client secret"), { target: { value: "shh" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Google" }));

    await waitFor(() => expect(googleStart).toHaveBeenCalledWith("abc.apps.googleusercontent.com", "shh", "loopback"));
    // The page does not leave: it links to Google and waits for the pasted address.
    const open = await screen.findByRole("link", { name: "Open Google's sign-in page" });
    expect(open.getAttribute("href")).toBe(AUTH_URL);
    expect(open.getAttribute("target")).toBe("_blank");
    expect(assign).not.toHaveBeenCalled();
    const pasted = "http://127.0.0.1:53682/?state=s1&code=c1&scope=email";
    fireEvent.input(screen.getByLabelText("Address from the browser"), { target: { value: pasted } });
    fireEvent.click(screen.getByRole("button", { name: "Finish connecting" }));

    await waitFor(() => expect(googleFinish).toHaveBeenCalledWith(pasted));
    expect(await screen.findByText("Connected as me@example.com")).toBeTruthy();
    expect(screen.queryByLabelText("Address from the browser")).toBeNull();
  });

  it("shows Google's refusal of a pasted address and keeps the paste box", async () => {
    googleStatus.mockResolvedValue(disconnected);
    googleStart.mockResolvedValue({ auth_url: AUTH_URL });
    const { ApiError } = await import("../api");
    googleFinish.mockRejectedValue(new ApiError(400, { error: "google", message: "Google answered invalid_grant: Bad Request" }));
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    fireEvent.input(await screen.findByLabelText("Client ID"), { target: { value: "abc" } });
    fireEvent.input(screen.getByLabelText("Client secret"), { target: { value: "shh" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Google" }));
    fireEvent.input(await screen.findByLabelText("Address from the browser"), { target: { value: "http://127.0.0.1:53682/?code=x" } });
    fireEvent.click(screen.getByRole("button", { name: "Finish connecting" }));
    expect((await screen.findByRole("alert")).textContent).toContain("invalid_grant");
    expect(screen.getByLabelText("Address from the browser")).toBeTruthy();
  });

  it("sends the browser straight to Google on https, with no paste box", async () => {
    googleStatus.mockResolvedValue(disconnected);
    googleStart.mockResolvedValue({ auth_url: AUTH_URL });
    const assign = vi.fn();
    render(() => <GoogleCard protocol="https:" assign={assign} />);
    fireEvent.input(await screen.findByLabelText("Client ID"), { target: { value: "abc" } });
    fireEvent.input(screen.getByLabelText("Client secret"), { target: { value: "shh" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Google" }));
    await waitFor(() => expect(googleStart).toHaveBeenCalledWith("abc", "shh", "direct"));
    await waitFor(() => expect(assign).toHaveBeenCalledWith(AUTH_URL));
    expect(screen.queryByLabelText("Address from the browser")).toBeNull();
  });

  it("asks for both halves of the client before starting", async () => {
    googleStatus.mockResolvedValue(disconnected);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    fireEvent.input(await screen.findByLabelText("Client ID"), { target: { value: "abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Google" }));
    expect((await screen.findByRole("alert")).textContent).toContain("client secret");
    expect(googleStart).not.toHaveBeenCalled();
  });

  it("shows the connected account, each side's last run and error, Run now and Disconnect", async () => {
    googleStatus.mockResolvedValue(connected);
    runTask.mockResolvedValue({ outcome: "succeeded" });
    googleDisconnect.mockResolvedValue({ ...disconnected, gmail: side("google_gmail", { last_run: "2026-10-18T09:00:00Z" }) });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);

    expect(await screen.findByText("Connected as me@example.com")).toBeTruthy();
    expect(screen.queryByLabelText("Client ID")).toBeNull();
    const gmail = screen.getByRole("row", { name: /Gmail/ });
    expect(gmail.textContent).toContain(new Date("2026-10-18T09:00:00Z").toLocaleString());
    expect(gmail.textContent).toContain("No errors");
    const calendar = screen.getByRole("row", { name: /Calendar/ });
    expect(calendar.textContent).toContain("Never");
    expect(calendar.textContent).toContain("Google answered 500: backend error");

    fireEvent.click(screen.getByRole("button", { name: "Run Gmail now" }));
    await waitFor(() => expect(runTask).toHaveBeenCalledWith("google_gmail"));
    expect(await screen.findByText("Gmail ran.")).toBeTruthy();
    await waitFor(() => expect((screen.getByRole("button", { name: "Run Calendar now" }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "Run Calendar now" }));
    await waitFor(() => expect(runTask).toHaveBeenCalledWith("google_calendar"));
    expect(await screen.findByText("Calendar ran.")).toBeTruthy();
    await waitFor(() => expect((screen.getByRole("button", { name: "Disconnect" }) as HTMLButtonElement).disabled).toBe(false));

    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(googleDisconnect).toHaveBeenCalled());
    expect(confirm).toHaveBeenCalled();
    expect(await screen.findByRole("button", { name: "Connect Google" })).toBeTruthy();
    expect(screen.queryByText(/Connected as/)).toBeNull();
    expect(screen.getByRole("row", { name: /Gmail/ }).textContent).toContain("Paused");
    confirm.mockRestore();
  });

  it("keeps the connection when Disconnect is not confirmed", async () => {
    googleStatus.mockResolvedValue(connected);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(googleDisconnect).not.toHaveBeenCalled();
    confirm.mockRestore();
  });
});

describe("GoogleSettingsForm", () => {
  /** Type `text` into the chip box labelled `label` and press Enter. */
  function addChip(label: string, text: string) {
    fireEvent.click(screen.getByRole("button", { name: `Add to ${label}` }));
    const box = screen.getByRole("combobox", { name: `Add to ${label}` }) as HTMLInputElement;
    box.value = text;
    fireEvent.input(box);
    fireEvent.keyDown(box, { key: "Enter" });
  }

  it("shows the saved settings and saves the edited ones", async () => {
    googleStatus.mockResolvedValue(connected);
    const saved: GoogleSettings = {
      gmail_mode: "labels",
      gmail_labels: ["INBOX", "Work/Clients"],
      gmail_window_days: 30,
      gmail_backfill_to: "2025-01-31",
      calendars: ["primary", "team@group.calendar.google.com"],
      default_calendar: "team@group.calendar.google.com",
    };
    googleSaveSettings.mockResolvedValue({ ...connected, settings: saved });
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);

    const form = await screen.findByRole("form", { name: "Google sync settings" });
    expect((screen.getByLabelText("Gmail mode") as HTMLSelectElement).value).toBe("labels");
    expect(screen.getByRole("list", { name: "Gmail labels values" }).textContent).toContain("INBOX");
    expect((screen.getByLabelText("Window (days)") as HTMLInputElement).value).toBe("90");
    expect(screen.getByRole("list", { name: "Calendars values" }).textContent).toContain("primary");

    addChip("Gmail labels", "Work/Clients");
    fireEvent.input(screen.getByLabelText("Window (days)"), { target: { value: "30" } });
    fireEvent.input(screen.getByLabelText("Back-fill to"), { target: { value: "2025-01-31" } });
    addChip("Calendars", "team@group.calendar.google.com");
    fireEvent.input(screen.getByLabelText("Default calendar"), { target: { value: "team@group.calendar.google.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(googleSaveSettings).toHaveBeenCalledWith(saved));
    expect(await screen.findByText("Saved. The next runs use these settings.")).toBeTruthy();
    expect(form).toBeTruthy();
  });

  it("saves all mail with no back-fill date, and refuses a window out of range or no calendar", async () => {
    googleStatus.mockResolvedValue(disconnected);
    googleSaveSettings.mockResolvedValue(disconnected);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    await screen.findByRole("form", { name: "Google sync settings" });

    fireEvent.input(screen.getByLabelText("Window (days)"), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toContain("1 to 3650");
    fireEvent.input(screen.getByLabelText("Window (days)"), { target: { value: "90" } });
    fireEvent.click(screen.getByRole("button", { name: "Remove primary" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toContain("at least one calendar");
    expect(googleSaveSettings).not.toHaveBeenCalled();

    addChip("Calendars", "primary");
    fireEvent.change(screen.getByLabelText("Gmail mode"), { target: { value: "all" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(googleSaveSettings).toHaveBeenCalledWith({ ...defaults, gmail_mode: "all" }),
    );
  });

  it("shows why the server refused the settings", async () => {
    googleStatus.mockResolvedValue(connected);
    const { ApiError } = await import("../api");
    googleSaveSettings.mockRejectedValue(
      new ApiError(400, { error: "bad_request", message: "the back-fill date must be a date as YYYY-MM-DD" }),
    );
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    await screen.findByRole("form", { name: "Google sync settings" });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toContain("YYYY-MM-DD");
  });
});

describe("GoogleCallback", () => {
  it("posts its own address to finish and returns to the Integrations page", async () => {
    googleFinish.mockResolvedValue(connected);
    const ws = M.emptyWorkspace("/integrations/google/callback?state=s1&code=c1&scope=email");
    const pane = M.activePane(ws);
    resetWorkspace(ws);
    render(() => (
      <PaneRouter pane={pane}>
        <Route path="/integrations/google/callback" component={GoogleCallback} />
        <Route path="/settings/integrations" component={() => <p>integrations page</p>} />
      </PaneRouter>
    ));
    expect(await screen.findByText("integrations page")).toBeTruthy();
    expect(googleFinish).toHaveBeenCalledTimes(1);
    const url = new URL(googleFinish.mock.calls[0]![0] as string);
    expect(url.pathname).toBe("/integrations/google/callback");
    expect(url.searchParams.get("state")).toBe("s1");
    expect(url.searchParams.get("code")).toBe("c1");
    // The callback is replaced, so Back does not post the used code again.
    expect(workspace().panes[pane]!.history).toEqual(["/settings/integrations"]);
  });

  it("shows why finishing failed, with a way back", async () => {
    const { ApiError } = await import("../api");
    googleFinish.mockRejectedValue(new ApiError(400, { error: "bad_request", message: "this sign-in was not started here" }));
    const ws = M.emptyWorkspace("/integrations/google/callback?state=old&code=c1");
    resetWorkspace(ws);
    render(() => (
      <PaneRouter pane={M.activePane(ws)}>
        <Route path="/integrations/google/callback" component={GoogleCallback} />
      </PaneRouter>
    ));
    expect((await screen.findByRole("alert")).textContent).toContain("not started here");
    expect(screen.getByRole("link", { name: /Integrations/ }).getAttribute("href")).toBe("/settings/integrations");
  });
});
