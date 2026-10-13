// The colour theme, kept in this browser. It starts from the system preference until the
// user picks one, and is applied as `data-theme` on <html> so CSS variables switch with it.
import { createSignal } from "solid-js";

export type Theme = "light" | "dark";

const KEY = "thevault.theme";

function stored(): Theme | null {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch {
    return null;
  }
}

function systemTheme(): Theme {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const [theme, setThemeSignal] = createSignal<Theme>(stored() ?? systemTheme());

function apply(t: Theme) {
  document.documentElement.dataset.theme = t;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", t === "dark" ? "#0b1120" : "#1e293b");
}

/** Apply the current theme to the page. Call once before the app renders. */
export function initTheme() {
  apply(theme());
}

/** Switch to `t` and remember it in this browser. */
export function setTheme(t: Theme) {
  setThemeSignal(t);
  apply(t);
  try {
    localStorage.setItem(KEY, t);
  } catch {
    // A browser that refuses storage still switches for this visit.
  }
}

export function toggleTheme() {
  setTheme(theme() === "dark" ? "light" : "dark");
}

export { theme };
