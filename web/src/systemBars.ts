// The Android app's system bars (ticket 79). Android 15 draws the app edge to edge, under the
// status bar and the gesture bar, and its webview reports those as env(safe-area-inset-*) only
// in some versions. The app's `VaultSystemBars` bridge hands the page the insets instead, as the
// CSS variables --sat, --sar, --sab and --sal on <html>, which styles.css pads the shell by. The
// bridge sends each change itself; this reads the insets it holds when the page starts, and
// tells it the theme so the status bar's icons stay readable. In a browser there is no bridge,
// the variables stay unset and every inset is 0.

interface SystemBarsBridge {
  insets(): string;
  setDark(dark: boolean): void;
}

function bridge(): SystemBarsBridge | undefined {
  return (window as unknown as { VaultSystemBars?: SystemBarsBridge }).VaultSystemBars;
}

/** Copy the insets the app holds onto the page's CSS variables. */
export function initSystemBars() {
  const b = bridge();
  if (!b) return;
  try {
    const insets = JSON.parse(b.insets()) as Record<"top" | "right" | "bottom" | "left", number>;
    const style = document.documentElement.style;
    style.setProperty("--sat", `${insets.top}px`);
    style.setProperty("--sar", `${insets.right}px`);
    style.setProperty("--sab", `${insets.bottom}px`);
    style.setProperty("--sal", `${insets.left}px`);
  } catch {
    // A bridge that answers badly leaves the page at insets 0, as in a browser.
  }
}

/** Light status bar icons over a dark page, dark ones over a light page. */
export function systemBarsTheme(dark: boolean) {
  try {
    bridge()?.setDark(dark);
  } catch {
    // Nothing to colour outside the app.
  }
}
