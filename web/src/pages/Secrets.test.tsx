import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";

const putLoginSecret = vi.fn();
const putSecret = vi.fn();
const secrets = vi.fn().mockResolvedValue([]);
vi.mock("../inApp", () => ({ appInfo: vi.fn().mockResolvedValue(null) }));
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, secrets, putLoginSecret, putSecret } };
});
const { Secrets } = await import("./Access");
afterEach(() => { cleanup(); vi.clearAllMocks(); secrets.mockResolvedValue([]); });

it("writes a Login with masked credentials and warns about unrestricted sites", async () => {
  putLoginSecret.mockResolvedValue({});
  render(() => <Secrets />);
  fireEvent.change(await screen.findByLabelText("Kind"), { target: { value: "login" } });
  expect(screen.getByText(/No origins means any site/)).toBeTruthy();
  fireEvent.input(screen.getByLabelText("Name"), { target: { value: "site" } });
  fireEvent.input(screen.getByLabelText("User name"), { target: { value: "test-user" } });
  const password = screen.getByLabelText("Password") as HTMLInputElement;
  const seed = screen.getByLabelText("One-time-code seed (optional)") as HTMLInputElement;
  expect(password.type).toBe("password");
  expect(seed.type).toBe("password");
  fireEvent.input(password, { target: { value: "made-up-password" } });
  fireEvent.input(seed, { target: { value: "JBSWY3DPEHPK3PXP" } });
  fireEvent.click(screen.getByRole("button", { name: "Save secret" }));
  await waitFor(() => expect(putLoginSecret).toHaveBeenCalledWith("site", {
    username: "test-user", password: "made-up-password", totp_seed: "JBSWY3DPEHPK3PXP", origins: [],
  }));
  await waitFor(() => expect(password.value).toBe(""));
  expect(seed.value).toBe("");
});

it("keeps string-secret writes unchanged", async () => {
  putSecret.mockResolvedValue({});
  render(() => <Secrets />);
  fireEvent.input(await screen.findByLabelText("Name"), { target: { value: "plain" } });
  fireEvent.input(screen.getByLabelText("Value"), { target: { value: "made-up-string" } });
  fireEvent.click(screen.getByRole("button", { name: "Save secret" }));
  await waitFor(() => expect(putSecret).toHaveBeenCalledWith("plain", "made-up-string"));
  expect(putLoginSecret).not.toHaveBeenCalled();
});

it("warns on an unrestricted Login in the list and replacement requires the password again", async () => {
  secrets.mockResolvedValue([{ name: "site", kind: "login", username: "test-user", origins: [], has_totp: true, created: 1, updated: 2 }]);
  render(() => <Secrets />);
  expect(await screen.findByText("Warning: any site (no origins)")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  expect((screen.getByLabelText("Kind") as HTMLSelectElement).value).toBe("login");
  expect((screen.getByLabelText("User name") as HTMLInputElement).value).toBe("test-user");
  const password = screen.getByLabelText("Password") as HTMLInputElement;
  expect(password.value).toBe("");
  expect(password.required).toBe(true);
  expect((screen.getByLabelText("One-time-code seed (optional)") as HTMLInputElement).value).toBe("");
});
