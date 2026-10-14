import { afterEach, describe, expect, it, vi } from "vitest";
import { copyText } from "./CodeBlock";

describe("copyText", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("uses the Clipboard API in a secure context", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    vi.stubGlobal("isSecureContext", true);
    expect(await copyText("let x = 1;")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("let x = 1;");
  });

  it("falls back to a hidden textarea over plain HTTP", async () => {
    vi.stubGlobal("isSecureContext", false);
    let copied: string | null = null;
    document.execCommand = vi.fn(() => {
      copied = document.querySelector("textarea")?.value ?? null;
      return true;
    });
    expect(await copyText("over http")).toBe(true);
    expect(copied).toBe("over http");
    expect(document.querySelector("textarea")).toBeNull();
  });
});
