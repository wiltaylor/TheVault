// The command palette's pure parts: fuzzy matching, what the typed text asks for (prefix
// modes), the note query it runs, and how notes and actions are ranked.

import type { TypeDef } from "../api";
import { lit } from "../search/searchModel";

// ---------------------------------------------------------------------------------------------
// Fuzzy matching

export interface Match {
  /** Higher is better. Only comparable between matches of the same query. */
  score: number;
  /** The indexes in the text of the characters the query matched, ascending. */
  positions: number[];
}

const SEP = /[\s\-_/.:,()[\]"'“”]/;

function boundary(text: string, j: number): number {
  if (j === 0) return 10;
  const prev = text[j - 1]!;
  if (SEP.test(prev)) return 8;
  const c = text[j]!;
  if (c !== c.toLowerCase() && prev === prev.toLowerCase() && prev !== prev.toUpperCase()) return 7;
  return 0;
}

/** The words of `s`, lower case. */
export function words(s: string): string[] {
  return s.toLowerCase().split(/\s+/).filter(Boolean);
}

/**
 * How well `query` matches `text`: its characters (spaces aside) in order, case ignored, scored
 * higher for starts of words, runs of adjacent characters, a prefix, and the whole text. Null
 * when the characters do not appear in order.
 */
export function fuzzy(query: string, text: string): Match | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (!q) return { score: 0, positions: [] };
  const t = text.toLowerCase();
  const m = q.length;
  const n = t.length;
  if (m > n) return null;
  // Quick reject: not a subsequence.
  for (let i = 0, j = 0; i < m; i++, j++) {
    j = t.indexOf(q[i]!, j);
    if (j < 0) return null;
  }
  // best[i*n + j]: the best score for q[0..i] with q[i] matched at t[j]; from[] remembers where
  // q[i-1] was, to read the positions back.
  const NEG = -Infinity;
  const best = new Float64Array(m * n).fill(NEG);
  const from = new Int32Array(m * n).fill(-1);
  for (let j = 0; j < n; j++) {
    if (t[j] === q[0]) best[j] = 1 + boundary(text, j) - Math.min(j, 20) * 0.1;
  }
  for (let i = 1; i < m; i++) {
    // The best of best[i-1][k] + 0.2k over k < j - 1, for a gap penalty of 0.2 a character.
    let run = NEG;
    let runAt = -1;
    const row = (i - 1) * n;
    for (let j = 1; j < n; j++) {
      if (j >= 2) {
        const k = j - 2;
        const v = best[row + k]! + 0.2 * k;
        if (v > run) {
          run = v;
          runAt = k;
        }
      }
      if (t[j] !== q[i]) continue;
      const bonus = 1 + boundary(text, j);
      const adjacent = best[row + j - 1]! + 5;
      const gapped = run - 0.2 * (j - 1);
      if (adjacent === NEG && gapped === NEG) continue;
      if (adjacent >= gapped) {
        best[i * n + j] = bonus + adjacent;
        from[i * n + j] = j - 1;
      } else {
        best[i * n + j] = bonus + gapped;
        from[i * n + j] = runAt;
      }
    }
  }
  let end = -1;
  let score = NEG;
  for (let j = 0; j < n; j++) {
    const v = best[(m - 1) * n + j]!;
    if (v > score) {
      score = v;
      end = j;
    }
  }
  if (end < 0) return null;
  const positions: number[] = new Array(m);
  for (let i = m - 1, j = end; i >= 0; i--) {
    positions[i] = j;
    j = from[i * n + j]!;
  }
  const whole = query.trim().toLowerCase().replace(/\s+/g, " ");
  if (t === whole) score += 100;
  else if (t.startsWith(whole)) score += 50;
  else if (t.includes(whole)) score += 20;
  return { score: score - n * 0.01, positions };
}

/** `text` cut into runs that `positions` (ascending) do and do not cover, for highlighting. */
export function segments(text: string, positions: readonly number[]): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  const hits = new Set(positions);
  for (let i = 0; i < text.length; i++) {
    const hit = hits.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.text += text[i];
    else out.push({ text: text[i]!, hit });
  }
  return out;
}

/** Where each word of `query` first starts a word of `text`, else occurs in it; for highlighting a title. */
export function wordPositions(query: string, text: string): number[] {
  const t = text.toLowerCase();
  const out = new Set<number>();
  for (const w of words(query)) {
    let at = -1;
    for (let i = t.indexOf(w); i >= 0; i = t.indexOf(w, i + 1)) {
      if (i === 0 || SEP.test(t[i - 1]!)) {
        at = i;
        break;
      }
      if (at < 0) at = i;
    }
    if (at >= 0) for (let k = 0; k < w.length; k++) out.add(at + k);
  }
  return [...out].sort((a, b) => a - b);
}

// ---------------------------------------------------------------------------------------------
// What the typed text asks for

export type Mode = "all" | "actions" | "type";

export interface Parsed {
  mode: Mode;
  /** The text to match, leading space removed; a trailing space says the last word is complete. */
  text: string;
  /** In `type` mode, the type word typed after `#` or `type:`. */
  typeWord: string;
}

/** `>` asks for actions only; `#word` or `type:word` for notes of the type `word` names. */
export function parseInput(raw: string): Parsed {
  const s = raw.replace(/^\s+/, "");
  if (s.startsWith(">")) return { mode: "actions", text: s.slice(1).replace(/^\s+/, ""), typeWord: "" };
  const m = /^(?:#|type:)(\S*)(\s.*)?$/is.exec(s);
  if (m) return { mode: "type", typeWord: m[1]!, text: (m[2] ?? "").replace(/^\s+/, "") };
  return { mode: "all", text: s, typeWord: "" };
}

type Named = Pick<TypeDef, "name" | "label" | "plural">;

/** The type `word` names: by name, label or plural, else the best fuzzy match. Null for none. */
export function resolveType<T extends Named>(types: readonly T[], word: string): T | null {
  const w = word.trim().toLowerCase();
  if (!w) return null;
  const exact = types.find((t) => [t.name, t.label, t.plural].some((x) => x?.toLowerCase() === w));
  if (exact) return exact;
  let found: T | null = null;
  let score = -Infinity;
  for (const t of types) {
    for (const x of [t.label, t.plural, t.name]) {
      const m = x ? fuzzy(w, x) : null;
      if (m && m.score > score) {
        score = m.score;
        found = t;
      }
    }
  }
  return found;
}

/**
 * The query for the notes the palette lists, or null when it lists only recent notes. The last
 * word matches as a prefix until a space follows it, as on the Search page.
 */
export function notesQuery(text: string, type: string | null): string | null {
  const t = text.trim();
  if (!t) return type ? `from ${type} sort updated desc select updated` : null;
  return `from ${type ?? "*"} search ${lit(/\s$/.test(text) ? `${t} ` : t)} select updated`;
}

// ---------------------------------------------------------------------------------------------
// Ranking

export interface NoteHit {
  id: string;
  title: string;
  type: string;
  /** When the note last changed, as the API gives it, if known. */
  updated?: string | null;
}

export interface RankedNote extends NoteHit {
  recent: boolean;
  positions: number[];
}

/** Whether every word of `query` occurs in `title`, case ignored. */
export function titleHas(query: string, title: string): boolean {
  const t = title.toLowerCase();
  const ws = words(query);
  return ws.length > 0 && ws.every((w) => t.includes(w));
}

function tier(q: string, ws: string[], title: string, recent: boolean): number {
  const t = title.toLowerCase().trim();
  if (t === q) return 0;
  if (t.startsWith(q)) return 1;
  const tw = t.split(SEP).filter(Boolean);
  if (ws.length && ws.every((w) => tw.some((x) => x.startsWith(w)))) return 2;
  return recent ? 3 : 4;
}

/**
 * `found` (in the server's order) and the recent notes whose titles hold the text, one entry per
 * note: exact title matches first, then titles that start with the text, then titles whose words
 * start with its words, then recent notes, then the rest. Recent notes come first within a rank.
 */
export function rankNotes(
  text: string,
  found: readonly NoteHit[],
  recents: readonly NoteHit[],
  limit: number,
): RankedNote[] {
  const q = text.trim().toLowerCase().replace(/\s+/g, " ");
  const ws = words(q);
  const recentAt = new Map(recents.map((r, i) => [r.id, i]));
  const seen = new Map<string, { hit: NoteHit; order: number }>();
  found.forEach((hit, i) => {
    if (!seen.has(hit.id)) seen.set(hit.id, { hit, order: i });
  });
  recents.forEach((hit, i) => {
    if (!seen.has(hit.id) && titleHas(q, hit.title)) seen.set(hit.id, { hit, order: found.length + i });
  });
  return [...seen.values()]
    .map(({ hit, order }) => {
      const r = recentAt.get(hit.id);
      return { hit, order, recent: r !== undefined, r: r ?? Infinity, tier: tier(q, ws, hit.title, r !== undefined) };
    })
    .sort((a, b) => a.tier - b.tier || a.r - b.r || a.order - b.order)
    .slice(0, limit)
    .map(({ hit, recent }) => ({ ...hit, recent, positions: wordPositions(q, hit.title) }));
}

export interface ActionLike {
  label: string;
  /** Other words the action answers to, such as a type's plural. */
  keywords?: readonly string[];
  /** Listed with nothing typed; the per-type actions are not, as there may be hundreds. */
  general?: boolean;
}

export interface RankedAction<A> {
  action: A;
  score: number;
  /** Characters of the label to highlight. */
  positions: number[];
  /** Every typed word starts a word of the label: a match the user most likely meant. */
  strong: boolean;
}

/**
 * The actions that match `text`, best first. With nothing typed, the general actions in their
 * order, then (when `all`) the rest.
 */
export function rankActions<A extends ActionLike>(actions: readonly A[], text: string, limit: number, all = false): RankedAction<A>[] {
  const q = text.trim();
  if (!q) {
    const list = all ? [...actions.filter((a) => a.general), ...actions.filter((a) => !a.general)] : actions.filter((a) => a.general);
    return list.slice(0, limit).map((action) => ({ action, score: 0, positions: [], strong: false }));
  }
  const ws = words(q);
  const minScore = q.replace(/\s+/g, "").length * 2;
  const out: (RankedAction<A> & { i: number })[] = [];
  actions.forEach((action, i) => {
    const m = fuzzy(q, action.label);
    let score = m && m.score > 0 ? m.score : -Infinity;
    for (const k of action.keywords ?? []) {
      const km = fuzzy(q, k);
      // A keyword counts for less than the label, so the label the user reads wins a tie.
      if (km && km.score > 0) score = Math.max(score, km.score * 0.6);
    }
    // Characters scattered through the label, none at the start of a word, are noise.
    if (score < minScore) return;
    const lw = action.label.toLowerCase().split(SEP).filter(Boolean);
    const strong = ws.every((w) => lw.some((x) => x.startsWith(w)));
    out.push({ action, score: score + (strong ? 30 : 0), positions: m?.positions ?? [], strong, i });
  });
  return out
    .sort((a, b) => b.score - a.score || a.i - b.i)
    .slice(0, limit)
    .map(({ i: _, ...rest }) => rest);
}
