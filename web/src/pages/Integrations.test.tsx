import { Route } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GoogleSettings, GoogleSide, GoogleStatus } from "../api";

const googleStatus = vi.fn();
const googleStart = vi.fn();
const googleFinish = vi.fn();
const googleDisconnect = vi.fn();
const runTask = vi.fn();
const taskRun = vi.fn();
const googleSaveSettings = vi.fn();
const braveStatus = vi.fn();
const bravePutKey = vi.fn();
const braveTest = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      googleStatus,
      googleStart,
      googleFinish,
      googleDisconnect,
      googleSaveSettings,
      runTask,
      taskRun,
      braveStatus,
      bravePutKey,
      braveTest,
    },
  };
});

const { BraveCard, GoogleCard, GoogleCallback, redirectMode } = await import("./Integrations");
const M = await import("../workspace/model");
const { PaneRouter } = await import("../workspace/pane");
const { resetWorkspace, workspace } = await import("../workspace/store");

function side(task: string, over: Partial<GoogleSide> = {}): GoogleSide {
  return { task, installed: true, paused: true, running: false, last_run: null, last_error: null, summary: null, ...over };
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

/** A promise and the functions that settle it, so a test sees the request in flight. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Whether `button` shows its request in flight: disabled, aria-busy and a turning glyph. */
function inFlight(button: HTMLElement): boolean {
  return (
    (button as HTMLButtonElement).disabled &&
    button.getAttribute("aria-busy") === "true" &&
    button.querySelector("svg.spinner") !== null
  );
}

function idle(button: HTMLElement): boolean {
  return button.getAttribute("aria-busy") === null && button.querySelector("svg.spinner") === null;
}

describe("GoogleCard feedback", () => {
  it("spins Connect Google while the start is in flight, and shows its error inline", async () => {
    googleStatus.mockResolvedValue(disconnected);
    const start = deferred<{ auth_url: string }>();
    googleStart.mockReturnValue(start.promise);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    fireEvent.input(await screen.findByLabelText("Client ID"), { target: { value: "abc" } });
    fireEvent.input(screen.getByLabelText("Client secret"), { target: { value: "shh" } });
    const button = screen.getByRole("button", { name: "Connect Google" });
    expect(idle(button)).toBe(true);
    fireEvent.click(button);
    await waitFor(() => expect(inFlight(button)).toBe(true));
    const { ApiError } = await import("../api");
    start.reject(new ApiError(500, { error: "internal", message: "the secrets key is missing" }));
    expect((await screen.findByRole("alert")).textContent).toContain("secrets key is missing");
    expect(idle(button)).toBe(true);
    expect((button as HTMLButtonElement).disabled).toBe(false);
  });

  it("spins Finish connecting, then shows the connected card with both sides syncing", async () => {
    googleStatus.mockResolvedValue(disconnected);
    googleStart.mockResolvedValue({ auth_url: AUTH_URL });
    const finish = deferred<GoogleStatus>();
    googleFinish.mockReturnValue(finish.promise);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} pollMs={60_000} />);
    fireEvent.input(await screen.findByLabelText("Client ID"), { target: { value: "abc" } });
    fireEvent.input(screen.getByLabelText("Client secret"), { target: { value: "shh" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Google" }));
    fireEvent.input(await screen.findByLabelText("Address from the browser"), {
      target: { value: "http://127.0.0.1:53682/?state=s1&code=c1" },
    });
    const button = screen.getByRole("button", { name: "Finish connecting" });
    fireEvent.click(button);
    await waitFor(() => expect(inFlight(button)).toBe(true));
    expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);

    finish.resolve({
      ...connected,
      gmail: { ...connected.gmail, running: true },
      calendar: { ...connected.calendar, running: true },
    });
    expect(await screen.findByText("Connected as me@example.com")).toBeTruthy();
    for (const name of [/Gmail/, /Calendar/]) {
      const row = screen.getByRole("row", { name });
      expect(row.textContent).toContain("Syncing…");
      expect(row.querySelector('[aria-busy="true"] svg.spinner')).not.toBeNull();
      expect((row.querySelector("button") as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("polls while a side runs, then shows its last run and summary", async () => {
    const done: GoogleStatus = {
      ...connected,
      gmail: { ...connected.gmail, running: false, last_run: "2026-10-18T10:00:00Z", summary: "Gmail: 0 new, 0 relabelled, 0 archived, 0 not saved." },
    };
    googleStatus
      .mockResolvedValueOnce({ ...connected, gmail: { ...connected.gmail, running: true } })
      .mockResolvedValue(done);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} pollMs={10} />);
    const gmail = await screen.findByRole("row", { name: /Gmail/ });
    await waitFor(() => expect(gmail.textContent).toContain("Syncing…"));
    // The calendar side is not running: its last run shows as before.
    expect(screen.getByRole("row", { name: /Calendar/ }).textContent).toContain("Never");

    await waitFor(() => expect(screen.getByRole("row", { name: /Gmail/ }).textContent).toContain("Gmail: 0 new, 0 relabelled, 0 archived"));
    const row = screen.getByRole("row", { name: /Gmail/ });
    expect(row.textContent).not.toContain("Syncing…");
    expect(row.textContent).toContain(new Date("2026-10-18T10:00:00Z").toLocaleString());
    // Nothing runs now, so the polling stops.
    const calls = googleStatus.mock.calls.length;
    await new Promise((r) => setTimeout(r, 60));
    expect(googleStatus.mock.calls.length).toBe(calls);
  });

  it("spins Run now and shows that side syncing while the run is in flight", async () => {
    googleStatus.mockResolvedValue(connected);
    const run = deferred<{ outcome: string; message?: string }>();
    runTask.mockReturnValue(run.promise);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} pollMs={60_000} />);
    const button = await screen.findByRole("button", { name: "Run Gmail now" });
    fireEvent.click(button);
    await waitFor(() => expect(inFlight(button)).toBe(true));
    expect(screen.getByRole("row", { name: /Gmail/ }).textContent).toContain("Syncing…");
    expect(screen.getByRole("row", { name: /Calendar/ }).textContent).not.toContain("Syncing…");
    run.resolve({ outcome: "failed", message: "Google answered 500" });
    expect(await screen.findByText("Gmail ran and failed: Google answered 500")).toBeTruthy();
    await waitFor(() => expect(idle(screen.getByRole("button", { name: "Run Gmail now" }))).toBe(true));
  });

  it("polls a started run until it ends, with the other buttons free meanwhile", async () => {
    googleStatus.mockResolvedValue(connected);
    const started = { id: 7, task: "google_gmail", outcome: "running", message: null };
    runTask.mockResolvedValue(started);
    taskRun.mockResolvedValueOnce(started).mockResolvedValueOnce({ ...started, outcome: "failed", message: "Google answered 500" });
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} pollMs={10} />);
    fireEvent.click(await screen.findByRole("button", { name: "Run Gmail now" }));
    await waitFor(() => expect(runTask).toHaveBeenCalledWith("google_gmail"));
    // The request has answered: Save is free while the run goes, and Gmail shows it syncing.
    await waitFor(() => expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole("row", { name: /Gmail/ }).textContent).toContain("Syncing…");
    expect(await screen.findByText("Gmail ran and failed: Google answered 500")).toBeTruthy();
    expect(taskRun).toHaveBeenCalledWith("google_gmail", 7);
    expect(taskRun).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(screen.getByRole("row", { name: /Gmail/ }).textContent).not.toContain("Syncing…"));
  });

  it("spins Save while the settings are saving, and shows a refusal inline", async () => {
    googleStatus.mockResolvedValue(connected);
    const save = deferred<GoogleStatus>();
    googleSaveSettings.mockReturnValue(save.promise);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    await screen.findByRole("form", { name: "Google sync settings" });
    const button = screen.getByRole("button", { name: "Save" });
    fireEvent.click(button);
    await waitFor(() => expect(inFlight(button)).toBe(true));
    const { ApiError } = await import("../api");
    save.reject(new ApiError(400, { error: "bad_request", message: "list at least one calendar" }));
    expect((await screen.findByRole("alert")).textContent).toContain("at least one calendar");
    expect(idle(button)).toBe(true);
  });

  it("spins Disconnect while it is in flight", async () => {
    googleStatus.mockResolvedValue(connected);
    const gone = deferred<GoogleStatus>();
    googleDisconnect.mockReturnValue(gone.promise);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <GoogleCard protocol="http:" assign={vi.fn()} />);
    const button = await screen.findByRole("button", { name: "Disconnect" });
    fireEvent.click(button);
    await waitFor(() => expect(inFlight(button)).toBe(true));
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    gone.resolve(disconnected);
    expect(await screen.findByRole("button", { name: "Connect Google" })).toBeTruthy();
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
    // Changing labels or mode lists the window again on the next run (google-sync D11, D21).
    expect(form.textContent).toContain("Changing the labels or the mode lists the window again on the next run.");

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

describe("BraveCard", () => {
  it("saves a key it never shows back, links to Brave's dashboard, and removes it", async () => {
    braveStatus.mockResolvedValue({ configured: false });
    bravePutKey.mockResolvedValueOnce({ configured: true }).mockResolvedValueOnce({ configured: false });
    const ok = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <BraveCard />);

    const link = await screen.findByRole("link", { name: /Brave Search API key/ });
    expect(link.getAttribute("href")).toBe("https://api-dashboard.search.brave.com/");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    const input = screen.getByLabelText("API key") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.placeholder).toBe("Paste a key");
    expect(screen.queryByRole("button", { name: "Test" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Remove key" })).toBeNull();

    fireEvent.input(input, { target: { value: " BSA-key " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(bravePutKey).toHaveBeenCalledWith("BSA-key"));
    await waitFor(() => expect(input.placeholder).toBe("set"));
    expect(input.value).toBe("");
    expect(screen.getByText("A key is set. Paste another to replace it.")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(bravePutKey).toHaveBeenLastCalledWith(""));
    await waitFor(() => expect(input.placeholder).toBe("Paste a key"));
    expect(ok).toHaveBeenCalled();
    ok.mockRestore();
  });

  it("tests the stored key and shows the result count or Brave's error", async () => {
    braveStatus.mockResolvedValue({ configured: true });
    braveTest
      .mockResolvedValueOnce({ ok: true, count: 7, ms: 120, error: null })
      .mockResolvedValueOnce({
        ok: false,
        count: null,
        ms: 30,
        error: { kind: "auth", message: "Brave answered 401 Unauthorized: The provided subscription token is invalid." },
      });
    render(() => <BraveCard />);

    fireEvent.click(await screen.findByRole("button", { name: "Test" }));
    expect((await screen.findByRole("status")).textContent).toContain("7 results");
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    expect((await screen.findByRole("alert")).textContent).toContain("subscription token is invalid");
    expect(braveTest).toHaveBeenCalledTimes(2);
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
