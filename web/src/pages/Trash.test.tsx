import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppInfo, TrashEntry } from "../api";

const trash = vi.fn();
const emptyTrash = vi.fn();
const purgeFromTrash = vi.fn();
const appInfo = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, trash, emptyTrash, purgeFromTrash } };
});
vi.mock("../inApp", () => ({ appInfo }));
vi.mock("@solidjs/router", () => ({ useNavigate: () => vi.fn() }));

const { Trash } = await import("./Trash");

const entry = (id: string, title: string, at: number): TrashEntry => ({
  id,
  type: "note",
  version: "v1",
  wcl: `title = "${title}"\n`,
  deleted_by: { kind: "session", name: "owner" },
  deleted_at: at,
});

const entries = [entry("01JA", "Old draft", 2), entry("01JB", "Shopping list", 1)];

beforeEach(() => {
  trash.mockResolvedValue(entries);
  appInfo.mockResolvedValue(null);
  emptyTrash.mockResolvedValue({ purged: ["01JA", "01JB"], assets_trashed: [] });
  purgeFromTrash.mockResolvedValue({ purged: ["01JA"], assets_trashed: [] });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Trash", () => {
  it("empties the trash after a confirm naming the count", async () => {
    render(() => <Trash />);
    fireEvent.click(await screen.findByRole("button", { name: "Empty trash" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toContain("Delete 2 notes in the trash forever");
    expect(emptyTrash).not.toHaveBeenCalled();
    trash.mockResolvedValue([]);
    const confirm = Array.from(dialog.querySelectorAll("button")).find((b) => b.textContent === "Empty trash");
    fireEvent.click(confirm!);
    await waitFor(() => expect(emptyTrash).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("The trash is empty.")).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("cancelling the confirm purges nothing", async () => {
    render(() => <Trash />);
    fireEvent.click(await screen.findByRole("button", { name: "Empty trash" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(emptyTrash).not.toHaveBeenCalled();
  });

  it("deletes one note forever after a confirm", async () => {
    render(() => <Trash />);
    fireEvent.click(await screen.findByRole("button", { name: "Delete Old draft forever" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toContain("Delete “Old draft” forever");
    expect(purgeFromTrash).not.toHaveBeenCalled();
    const confirm = Array.from(dialog.querySelectorAll("button")).find((b) => b.textContent === "Delete forever");
    fireEvent.click(confirm!);
    await waitFor(() => expect(purgeFromTrash).toHaveBeenCalledWith("01JA"));
    expect(emptyTrash).not.toHaveBeenCalled();
  });

  it("offers no purge inside the apps", async () => {
    appInfo.mockResolvedValue({ device_id: "d1" } as unknown as AppInfo);
    render(() => <Trash />);
    expect(await screen.findByText("Old draft")).toBeTruthy();
    await Promise.resolve();
    expect(screen.queryByRole("button", { name: "Empty trash" })).toBeNull();
    expect(screen.queryByRole("button", { name: /forever/ })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Restore" })).toHaveLength(2);
  });
});
