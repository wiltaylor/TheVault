import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ViewOriginalButton, hasRemoteImages, originalDocument, originalPolicy } from "./ViewOriginal";

const PAGE = '<p>Hello</p><img src="data:image/png;base64,AA" alt="inline"><img src="https://img.example.com/hero.png" alt="hero">';

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, originalHtml: vi.fn(async () => PAGE) };
});

describe("View original", () => {
  afterEach(cleanup);

  it("shows the original in a frame that runs no script and has no origin", async () => {
    render(() => <ViewOriginalButton id="01JB8Z4K2MQ7V3X9T5R6W8Y2ZA" />);
    fireEvent.click(screen.getByRole("button", { name: "View original" }));
    // The dialog sits in a portal, outside the rendered container.
    const frame = (await screen.findByTestId("original-frame")) as HTMLIFrameElement;
    const sandbox = frame.getAttribute("sandbox") ?? "";
    expect(sandbox).not.toContain("allow-scripts");
    expect(sandbox).not.toContain("allow-same-origin");
    expect(sandbox).not.toContain("allow-forms");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    const doc = frame.getAttribute("srcdoc") ?? "";
    // The policy comes first, before anything the email holds, and lets in only data: images.
    expect(doc.indexOf("Content-Security-Policy")).toBeLessThan(doc.indexOf("<p>Hello"));
    expect(doc).toContain(`content="${originalPolicy(false)}"`);
    expect(originalPolicy(false)).toContain("img-src data:;");
    expect(originalPolicy(false)).toContain("default-src 'none'");
  });

  it("blocks remote images until asked", async () => {
    render(() => <ViewOriginalButton id="01JB8Z4K2MQ7V3X9T5R6W8Y2ZA" />);
    fireEvent.click(screen.getByRole("button", { name: "View original" }));
    const frame = (await screen.findByTestId("original-frame")) as HTMLIFrameElement;
    expect(frame.getAttribute("srcdoc")).not.toContain("https:;");
    fireEvent.click(screen.getByRole("button", { name: "Load remote images" }));
    await waitFor(() => {
      const again = screen.getByTestId("original-frame");
      expect(again.getAttribute("srcdoc")).toContain("img-src data: https:;");
    });
    expect(screen.queryByRole("button", { name: "Load remote images" })).toBeNull();
  });

  it("knows a page with remote images", () => {
    expect(hasRemoteImages(PAGE)).toBe(true);
    expect(hasRemoteImages('<img src="data:image/png;base64,AA">')).toBe(false);
    expect(hasRemoteImages('<td style="background:url(https://e.example/a.png)">')).toBe(true);
    expect(originalDocument("<p>x</p>", true)).toContain("img-src data: https:");
  });
});
