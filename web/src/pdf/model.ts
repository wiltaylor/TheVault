// The PDF viewer's state that does not need pdf.js: the page layout, which pages are near the
// viewport and so get drawn, the page on show, zoom steps and find matches. PdfViewer.tsx drives
// the DOM from these.

/** The gap between pages and around them, in CSS pixels. */
export const PAGE_GAP = 12;

/** Zoom bounds and the factor one step of zoom in or out takes. */
export const ZOOM = { min: 0.25, max: 5, step: 1.25 };

/** An inline viewer's height in CSS pixels: the default and bounds (crates/schema, `ASSET_HEIGHT_*`). */
export const VIEWER_HEIGHT = { initial: 560, min: 160, max: 4000 };

export function clampHeight(h: number): number {
  return Math.round(Math.min(VIEWER_HEIGHT.max, Math.max(VIEWER_HEIGHT.min, h)));
}

/** A page's size at scale 1, in PDF points. */
export interface PageSize {
  width: number;
  height: number;
}

/**
 * Where each page sits at `scale`: its top and its height, in CSS pixels, and the whole height.
 * `start` is the first page's top, below the floating controls.
 */
export interface Layout {
  start: number;
  tops: number[];
  heights: number[];
  widths: number[];
  total: number;
}

/**
 * The layout of `count` pages at `scale`, the first `start` pixels down. A page whose size is
 * not known yet takes the size of the first page known, so the layout only shifts when a
 * differently sized page loads.
 */
export function layout(sizes: (PageSize | undefined)[], count: number, scale: number, start = PAGE_GAP): Layout {
  const fallback = sizes.find((s) => s) ?? { width: 612, height: 792 };
  const tops: number[] = [];
  const heights: number[] = [];
  const widths: number[] = [];
  let y = start;
  for (let i = 0; i < count; i++) {
    const s = sizes[i] ?? fallback;
    const h = Math.floor(s.height * scale);
    tops.push(y);
    heights.push(h);
    widths.push(Math.floor(s.width * scale));
    y += h + PAGE_GAP;
  }
  return { start, tops, heights, widths, total: y };
}

/** The first page index whose bottom is below `y`, by binary search. */
function firstBelow(l: Layout, y: number): number {
  let lo = 0;
  let hi = l.tops.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (l.tops[mid]! + l.heights[mid]! < y) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * The pages to draw for a viewport scrolled to `top` and `height` tall: every page it shows,
 * and `overscan` more on each side so scrolling finds them drawn. Returns 0-based indices,
 * first and last inclusive, or null when there are no pages.
 */
export function pagesNear(l: Layout, top: number, height: number, overscan = 1): { first: number; last: number } | null {
  const n = l.tops.length;
  if (n === 0) return null;
  const first = firstBelow(l, top);
  let last = first;
  while (last + 1 < n && l.tops[last + 1]! < top + height) last++;
  return { first: Math.max(0, first - overscan), last: Math.min(n - 1, last + overscan) };
}

/**
 * The page on show, 0-based: the one under a line a third of the way down the viewport (at most
 * 120 pixels below where the pages start, under the controls), so the page counts as reached once
 * it fills most of the view; or the last page, scrolled to the end.
 */
export function currentPage(l: Layout, top: number, height: number): number {
  if (l.tops.length === 0) return 0;
  // Scrolled to the end, the last page is on show though it cannot reach the line.
  if (top > 0 && top + height >= l.total - 1) return l.tops.length - 1;
  const line = top + l.start + Math.min(height / 3, 120);
  const i = firstBelow(l, line);
  // In the gap above page i, page i is the one coming.
  return Math.min(i, l.tops.length - 1);
}

/** The scroll position that puts page `index` where the first page starts, below the controls. */
export function scrollFor(l: Layout, index: number): number {
  const i = Math.max(0, Math.min(l.tops.length - 1, index));
  return Math.max(0, (l.tops[i] ?? 0) - l.start);
}

/** The scale that fits a page `pageWidth` points wide into `available` CSS pixels. */
export function fitScale(pageWidth: number, available: number): number {
  if (pageWidth <= 0 || available <= 0) return 1;
  return clampZoom((available - 2 * PAGE_GAP) / pageWidth);
}

export function clampZoom(z: number): number {
  return Math.min(ZOOM.max, Math.max(ZOOM.min, z));
}

/** One zoom step in (`1`) or out (`-1`) from `scale`. */
export function zoomStep(scale: number, dir: 1 | -1): number {
  return clampZoom(dir > 0 ? scale * ZOOM.step : scale / ZOOM.step);
}

/** The page indicator's text: "3 / 12". */
export function pageLabel(current: number, count: number): string {
  return count > 0 ? `${current + 1} / ${count}` : "– / –";
}

/** One find match: the page it is on, 0-based, and which match on that page it is. */
export interface Match {
  page: number;
  index: number;
}

/** Every match of `query` in the pages' text, case and white space insensitive, in page order. */
export function findMatches(pageTexts: string[], query: string): Match[] {
  const q = normalise(query);
  if (!q) return [];
  const out: Match[] = [];
  pageTexts.forEach((text, page) => {
    const t = normalise(text);
    let at = t.indexOf(q);
    let index = 0;
    while (at !== -1) {
      out.push({ page, index: index++ });
      at = t.indexOf(q, at + q.length);
    }
  });
  return out;
}

/** Text lowercased with every run of white space one space. */
export function normalise(s: string): string {
  return s.toLowerCase().replace(/\s+/g, " ").trim();
}

/** The match after (`1`) or before (`-1`) `at`, wrapping round, or -1 when there are none. */
export function stepMatch(count: number, at: number, dir: 1 | -1): number {
  if (count === 0) return -1;
  if (at < 0) return dir > 0 ? 0 : count - 1;
  return (at + dir + count) % count;
}

/** Whether an asset called `filename` is a PDF, by its extension (crates/index, `is_pdf`). */
export function isPdfFile(filename: string): boolean {
  return /\.pdf$/i.test(filename);
}
