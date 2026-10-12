// How the admin pages show times and authors.

import type { Author } from "../api";

/** A time in milliseconds since the epoch, in the browser's locale. */
export function when(ms: number | null | undefined): string {
  return ms === null || ms === undefined ? "—" : new Date(ms).toLocaleString();
}

export function authorText(a: Author): string {
  if (a.kind === "session") return "you";
  if (a.kind === "token") return `token ${a.name}`;
  return `handler ${a.name}`;
}

/** The title a note's WCL holds, for a note that is only available as text, such as in the trash. */
export function wclTitle(wcl: string): string | null {
  const m = /^title\s*=\s*"((?:[^"\\]|\\.)*)"/m.exec(wcl);
  if (!m) return null;
  try {
    return JSON.parse(`"${m[1]}"`) as string;
  } catch {
    return m[1]!;
  }
}
