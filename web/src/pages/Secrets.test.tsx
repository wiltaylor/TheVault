import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";

const putLoginSecret = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, secrets: vi.fn().mockResolvedValue([]), putLoginSecret } };
});
const { Secrets } = await import("./Access");
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it("writes a Login with masked credentials and warns about unrestricted sites", async () => {
  putLoginSecret.mockResolvedValue({});
  render(() => <Secrets />);
  fireEvent.change(screen.getByLabelText("Kind"), { target: { value: "login" } });
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
