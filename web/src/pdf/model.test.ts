import { describe, expect, it } from "vitest";
import {
  PAGE_GAP,
  VIEWER_HEIGHT,
  clampHeight,
  currentPage,
  findMatches,
  fitScale,
  isPdfFile,
  layout,
  pageLabel,
  pagesNear,
  scrollFor,
  stepMatch,
  zoomStep,
} from "./model";

const letter = { width: 612, height: 792 };

describe("layout", () => {
  it("stacks pages with a gap, sizing unknown pages like the first known", () => {
    const l = layout([letter], 3, 0.5);
    expect(l.heights).toEqual([396, 396, 396]);
    expect(l.tops).toEqual([PAGE_GAP, PAGE_GAP * 2 + 396, PAGE_GAP * 3 + 792]);
    expect(l.total).toBe(PAGE_GAP * 4 + 396 * 3);
    expect(l.widths).toEqual([306, 306, 306]);
  });

  it("uses each page's own size once known", () => {
    const l = layout([letter, { width: 792, height: 612 }], 2, 1);
    expect(l.heights).toEqual([792, 612]);
    expect(l.widths).toEqual([612, 792]);
  });
});

describe("virtualisation", () => {
  const l = layout([letter], 100, 1); // pages 792 tall, 804 apart

  it("draws only the pages in view and one either side", () => {
    expect(pagesNear(l, 0, 600)).toEqual({ first: 0, last: 1 });
    // Scrolled to the middle of page 50 (index 49), a view 600 tall shows it alone.
    const top = l.tops[49]! + 100;
    expect(pagesNear(l, top, 600)).toEqual({ first: 48, last: 50 });
    // A tall view showing the end of one page and the start of the next draws both, plus one each side.
    expect(pagesNear(l, l.tops[10]! + 700, 400)).toEqual({ first: 9, last: 12 });
    expect(pagesNear(l, l.total, 600)).toEqual({ first: 98, last: 99 });
  });

  it("draws nothing without pages", () => {
    expect(pagesNear(layout([], 0, 1), 0, 600)).toBeNull();
  });

  it("counts a page as current once it reaches a third of the way down", () => {
    expect(currentPage(l, 0, 600)).toBe(0);
    expect(currentPage(l, l.tops[3]! - 300, 600)).toBe(2);
    expect(currentPage(l, l.tops[3]! - 100, 600)).toBe(3);
    expect(currentPage(l, l.total, 600)).toBe(99);
    // Scrolled to the very end of three short pages, the last is current.
    const short = layout([letter], 3, 0.5);
    expect(currentPage(short, short.total - 600, 600)).toBe(2);
  });

  it("scrolls to a page's top, clamped to the pages there are", () => {
    expect(scrollFor(l, 0)).toBe(0);
    expect(scrollFor(l, 2)).toBe(l.tops[2]! - PAGE_GAP);
    expect(scrollFor(l, 500)).toBe(l.tops[99]! - PAGE_GAP);
  });
});

describe("controls", () => {
  it("labels the page as n / total", () => {
    expect(pageLabel(2, 12)).toBe("3 / 12");
    expect(pageLabel(0, 0)).toBe("– / –");
  });

  it("zooms in steps within bounds and fits the width", () => {
    expect(zoomStep(1, 1)).toBeCloseTo(1.25);
    expect(zoomStep(1, -1)).toBeCloseTo(0.8);
    expect(zoomStep(4.9, 1)).toBe(5);
    expect(zoomStep(0.26, -1)).toBe(0.25);
    expect(fitScale(612, 612 + 2 * PAGE_GAP)).toBeCloseTo(1);
    expect(fitScale(612, 0)).toBe(1);
    expect(fitScale(612, 100)).toBe(0.25);
  });

  it("clamps the viewer height", () => {
    expect(clampHeight(10)).toBe(VIEWER_HEIGHT.min);
    expect(clampHeight(99_999)).toBe(VIEWER_HEIGHT.max);
    expect(clampHeight(600.4)).toBe(600);
  });

  it("knows a PDF by its name", () => {
    expect(isPdfFile("Report.PDF")).toBe(true);
    expect(isPdfFile("report.pdf.txt")).toBe(false);
  });
});

describe("find", () => {
  const pages = ["Page one\nhas the Word", "nothing", "word word, WORD"];

  it("finds every match in page order, ignoring case and white space", () => {
    expect(findMatches(pages, "word")).toEqual([
      { page: 0, index: 0 },
      { page: 2, index: 0 },
      { page: 2, index: 1 },
      { page: 2, index: 2 },
    ]);
    expect(findMatches(pages, "one has")).toEqual([{ page: 0, index: 0 }]);
    expect(findMatches(pages, "  ")).toEqual([]);
  });

  it("steps through matches, wrapping round", () => {
    expect(stepMatch(3, -1, 1)).toBe(0);
    expect(stepMatch(3, -1, -1)).toBe(2);
    expect(stepMatch(3, 2, 1)).toBe(0);
    expect(stepMatch(3, 0, -1)).toBe(2);
    expect(stepMatch(0, 0, 1)).toBe(-1);
  });
});
