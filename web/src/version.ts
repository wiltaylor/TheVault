// The web client's build, baked in by vite.config.ts: the workspace version the server shares,
// and the short commit it was built from when git could say.

declare const __WEB_VERSION__: string;
declare const __WEB_COMMIT__: string | null;

/** The workspace version this web build was made from. */
export const WEB_VERSION: string = __WEB_VERSION__;

/** The short commit this web build was made from, or null when git could not say. */
export const WEB_COMMIT: string | null = __WEB_COMMIT__;

/** The web build as Settings > General shows it: `0.1.0 (abc1234)`, or the version alone. */
export function webBuild(): string {
  return WEB_COMMIT ? `${WEB_VERSION} (${WEB_COMMIT})` : WEB_VERSION;
}
