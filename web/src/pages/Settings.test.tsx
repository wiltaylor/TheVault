import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";

const agents = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, agents } };
});
vi.mock("./About", () => ({ About: () => <div /> }));
const { Settings } = await import("./Settings");
afterEach(cleanup);

it("shows reserved legacy agent warnings under Settings Agents", async () => {
  agents.mockResolvedValue({ agents: [], tools: [], warnings: ["Agent user uses a reserved name. Rename its file."] });
  render(() => <Settings />);
  expect(await screen.findByRole("region", { name: "Agents" })).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("reserved name");
});

it("has no Agents warning section for ordinary names", async () => {
  agents.mockResolvedValue({ agents: [], tools: [], warnings: [] });
  render(() => <Settings />);
  await waitFor(() => expect(agents).toHaveBeenCalled());
  expect(screen.queryByRole("region", { name: "Agents" })).toBeNull();
});
