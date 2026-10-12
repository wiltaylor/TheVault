import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { App } from "./App";

describe("App", () => {
  it("renders the app name", () => {
    render(() => <App />);
    expect(screen.getByRole("heading", { name: "TheVault" })).toBeInTheDocument();
  });
});
