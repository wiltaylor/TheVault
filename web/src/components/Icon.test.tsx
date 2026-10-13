import { render, waitFor } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { TypeIcon, pascalName } from "./Icon";

describe("type icons", () => {
  it("turns a vault icon name into Lucide's export name", () => {
    expect(pascalName("file-text")).toBe("FileText");
    expect(pascalName("check-square")).toBe("CheckSquare");
    expect(pascalName("user")).toBe("User");
  });

  it("draws the named icon as SVG shapes", async () => {
    const { container } = render(() => <TypeIcon name="user" />);
    await waitFor(() => expect(container.querySelectorAll("svg circle").length).toBe(1));
    const circle = container.querySelector("svg circle")!;
    expect(circle.namespaceURI).toBe("http://www.w3.org/2000/svg");
  });

  it("draws a default icon for an unknown name or no name", async () => {
    for (const name of ["no-such-icon", null]) {
      const { container, unmount } = render(() => <TypeIcon name={name} />);
      await waitFor(() => expect(container.querySelectorAll("svg path").length).toBeGreaterThan(0));
      unmount();
    }
  });
});
