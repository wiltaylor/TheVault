import { afterEach, describe, expect, it, vi } from "vitest";
import { initSystemBars, systemBarsTheme } from "./systemBars";

type Win = { VaultSystemBars?: unknown };

afterEach(() => {
  delete (window as unknown as Win).VaultSystemBars;
  document.documentElement.removeAttribute("style");
});

describe("system bars", () => {
  it("leaves a browser's insets unset", () => {
    initSystemBars();
    systemBarsTheme(true);
    expect(document.documentElement.style.getPropertyValue("--sat")).toBe("");
  });

  it("copies the app's insets onto the page and reports the theme", () => {
    const setDark = vi.fn();
    (window as unknown as Win).VaultSystemBars = {
      insets: () => '{"top":24,"right":0,"bottom":16,"left":0}',
      setDark,
    };
    initSystemBars();
    const style = document.documentElement.style;
    expect(style.getPropertyValue("--sat")).toBe("24px");
    expect(style.getPropertyValue("--sab")).toBe("16px");
    expect(style.getPropertyValue("--sal")).toBe("0px");
    systemBarsTheme(true);
    expect(setDark).toHaveBeenCalledWith(true);
  });
});
