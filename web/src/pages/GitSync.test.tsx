import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitSyncSettings, GitSyncStatus } from "../api";

const gitsyncStatus = vi.fn();
const gitsyncSaveSettings = vi.fn();
const gitsyncPutToken = vi.fn();
const gitsyncDeleteToken = vi.fn();
const gitsyncGenerateSshKey = vi.fn();
const gitsyncPutSshKey = vi.fn();
const gitsyncGenerateKey = vi.fn();
const gitsyncPutKey = vi.fn();
const gitsyncRun = vi.fn();
const gitsyncDisconnect = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      gitsyncStatus,
      gitsyncSaveSettings,
      gitsyncPutToken,
      gitsyncDeleteToken,
      gitsyncGenerateSshKey,
      gitsyncPutSshKey,
      gitsyncGenerateKey,
      gitsyncPutKey,
      gitsyncRun,
      gitsyncDisconnect,
    },
  };
});
vi.mock("../clipboard", () => ({ copyText: vi.fn(async () => true) }));

const { ApiError } = await import("../api");
const { GitSync, refusedField } = await import("./GitSync");
const { copyText } = await import("../clipboard");

const settings: GitSyncSettings = {
  remote_url: "https://github.com/me/vault-sync.git",
  auth: "https",
  https_username: "x-access-token",
  export_query: 'type = "project"',
  import_query: 'type = "project"',
  schedule: "*/15 * * * *",
  name: "laptop",
  warn_bytes: 1024 ** 3,
  max_bytes: 4 * 1024 ** 3,
  paused: false,
};

function status(over: Partial<GitSyncStatus> = {}): GitSyncStatus {
  return {
    configured: true,
    settings,
    token_set: true,
    ssh_key_set: false,
    ssh_public_key: null,
    host_fingerprint: null,
    key_set: true,
    running: false,
    last_run: null,
    last_error: null,
    last_report: null,
    ...over,
  };
}

const unconfigured = status({ configured: false, settings: null, token_set: false, key_set: false });

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("refusedField", () => {
  it("takes the field the 400 names, else the one its message mentions", () => {
    expect(refusedField(new ApiError(400, { message: "x", field: "import_query" }))).toBe("import_query");
    expect(refusedField(new ApiError(400, { message: "the export query does not parse: line 1" }))).toBe("export_query");
    expect(refusedField(new ApiError(400, { message: "the cron does not parse" }))).toBe("schedule");
    expect(refusedField(new ApiError(400, { message: "the remote URL is empty" }))).toBeNull();
    expect(refusedField(new ApiError(500, { message: "the import query" }))).toBeNull();
  });
});

describe("GitSync", () => {
  it("saves the form and reads the saved settings back", async () => {
    gitsyncStatus.mockResolvedValueOnce(unconfigured).mockResolvedValue(status());
    gitsyncSaveSettings.mockResolvedValue(status());
    render(() => <GitSync />);

    const remote = (await screen.findByLabelText("Remote URL")) as HTMLInputElement;
    expect(remote.value).toBe("");
    expect((screen.getByLabelText("Username") as HTMLInputElement).value).toBe("x-access-token");
    expect((screen.getByLabelText("Schedule (cron)") as HTMLInputElement).value).toBe("*/15 * * * *");
    fireEvent.input(remote, { target: { value: settings.remote_url } });
    fireEvent.input(screen.getByLabelText("Export query"), { target: { value: settings.export_query } });
    fireEvent.input(screen.getByLabelText("Import query"), { target: { value: settings.import_query } });
    fireEvent.input(screen.getByLabelText("Instance name"), { target: { value: "laptop" } });
    fireEvent.input(screen.getByLabelText("Warn above (MB)"), { target: { value: "1024" } });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() => expect(gitsyncSaveSettings).toHaveBeenCalledWith(settings));
    expect((await screen.findByRole("status")).textContent).toContain("Saved");
    expect(gitsyncStatus).toHaveBeenCalledTimes(2);
  });

  it("shows a refused query's message under that query", async () => {
    gitsyncStatus.mockResolvedValue(status());
    gitsyncSaveSettings.mockRejectedValue(
      new ApiError(400, { error: "bad_request", message: "the import query may not use this", field: "import_query" }),
    );
    render(() => <GitSync />);
    fireEvent.input(await screen.findByLabelText("Import query"), { target: { value: "id = this.id" } });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

    const shown = await screen.findByText("the import query may not use this");
    // Under the import query, and nowhere else.
    expect(shown.id).toBe("gitsync-import-error");
    expect(screen.getByLabelText("Import query").getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByLabelText("Export query").getAttribute("aria-invalid")).toBeNull();
    expect(screen.getAllByRole("alert")).toHaveLength(1);

    // A save that then works takes the message away.
    gitsyncSaveSettings.mockResolvedValue(status());
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(screen.queryByText("the import query may not use this")).toBeNull());
  });

  it("shows a generated sync key once, with a copy button, and not after a reload", async () => {
    gitsyncStatus.mockResolvedValueOnce(status({ key_set: false })).mockResolvedValue(status({ key_set: true }));
    gitsyncGenerateKey.mockResolvedValue({ key: "tvsync1-AAAABBBBCCCC" });
    const first = render(() => <GitSync />);

    expect(await screen.findByText("No sync key yet.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Generate key" }));
    const key = (await screen.findByLabelText("Generated sync key")) as HTMLInputElement;
    expect(key.value).toBe("tvsync1-AAAABBBBCCCC");
    expect(screen.getByRole("region", { name: "New sync key" }).textContent).toContain("not shown again");
    fireEvent.click(screen.getByRole("button", { name: "Copy sync key" }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith("tvsync1-AAAABBBBCCCC"));
    expect(await screen.findByText("A sync key is set.")).toBeTruthy();

    // A reload reads the status, which never carries the key.
    first.unmount();
    render(() => <GitSync />);
    expect(await screen.findByText("A sync key is set.")).toBeTruthy();
    expect(screen.queryByLabelText("Generated sync key")).toBeNull();
    expect(document.body.textContent).not.toContain("tvsync1-AAAABBBBCCCC");
    expect(gitsyncGenerateKey).toHaveBeenCalledTimes(1);
  });

  it("shows the token field for HTTPS, never its value, and the public key for SSH", async () => {
    const pub = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGitSync thevault-gitsync";
    gitsyncStatus.mockResolvedValue(
      status({ ssh_key_set: true, ssh_public_key: pub, host_fingerprint: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s" }),
    );
    gitsyncPutToken.mockResolvedValue(null);
    render(() => <GitSync />);

    const tokenField = (await screen.findByLabelText("Token")) as HTMLInputElement;
    expect(tokenField.type).toBe("password");
    expect(tokenField.value).toBe("");
    expect(tokenField.placeholder).toBe("set");
    expect(screen.getByText("A token is set. Paste another to replace it.")).toBeTruthy();
    expect(screen.queryByLabelText("Public key")).toBeNull();
    fireEvent.input(tokenField, { target: { value: " github_pat_123 " } });
    fireEvent.click(screen.getByRole("button", { name: "Save token" }));
    await waitFor(() => expect(gitsyncPutToken).toHaveBeenCalledWith("github_pat_123"));
    await waitFor(() => expect(tokenField.value).toBe(""));

    fireEvent.change(screen.getByLabelText("Sign in with"), { target: { value: "ssh" } });
    expect(screen.queryByLabelText("Token")).toBeNull();
    expect(screen.queryByLabelText("Username")).toBeNull();
    expect((screen.getByLabelText("Public key") as HTMLInputElement).value).toBe(pub);
    expect(screen.getByText(/SHA256:uNiVztks/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Copy public key" }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith(pub));
    expect(screen.getByLabelText("Or paste an OpenSSH private key")).toBeTruthy();
  });

  it("makes a key pair when SSH has none", async () => {
    const pub = "ssh-ed25519 AAAAnew thevault-gitsync";
    gitsyncStatus
      .mockResolvedValueOnce(status({ settings: { ...settings, auth: "ssh" } }))
      .mockResolvedValue(status({ settings: { ...settings, auth: "ssh" }, ssh_key_set: true, ssh_public_key: pub }));
    gitsyncGenerateSshKey.mockResolvedValue({ public_key: pub });
    render(() => <GitSync />);
    fireEvent.click(await screen.findByRole("button", { name: "Make a key pair" }));
    expect(((await screen.findByLabelText("Public key")) as HTMLInputElement).value).toBe(pub);
  });

  it("starts a round with Sync now and reads the status while it runs", async () => {
    gitsyncStatus
      .mockResolvedValueOnce(status())
      .mockResolvedValueOnce(status({ running: true }))
      .mockResolvedValue(
        status({
          last_run: "2026-11-03T09:00:00Z",
          last_report: { pushed: true, created: 2, updated: 1, merged: 0, clashes: [], archived: 0, unarchived: 0, bytes: 2048 },
        }),
      );
    gitsyncRun.mockResolvedValue(null);
    render(() => <GitSync pollMs={10} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sync now" }));

    await waitFor(() => expect(gitsyncRun).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/Syncing/)).toBeTruthy();
    expect(await screen.findByText(/Pushed\. 2 created, 1 updated, 0 merged, 0 clashes/)).toBeTruthy();
    expect(screen.getByText("The round finished.")).toBeTruthy();
    expect(screen.getByText(/2\.0 KB/)).toBeTruthy();
    expect(gitsyncStatus).toHaveBeenCalledTimes(3);
  });
});
