import { render } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { VirtualList } from "./VirtualList";

describe("VirtualList", () => {
  it("renders only the rows near the viewport of a long list", () => {
    const { container } = render(() => (
      <VirtualList count={10_000} rowHeight={40}>
        {(i) => <span>Row {i}</span>}
      </VirtualList>
    ));
    const rows = container.querySelectorAll("[role=listitem]");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(200);
    const body = container.querySelector(".virtual-list-body") as HTMLElement;
    expect(body.style.height).toBe("400000px");
  });

  it("reports the rendered range", () => {
    const ranges: [number, number][] = [];
    render(() => (
      <VirtualList count={5} rowHeight={40} onRange={(s, e) => ranges.push([s, e])}>
        {(i) => <span>Row {i}</span>}
      </VirtualList>
    ));
    expect(ranges.at(-1)).toEqual([0, 5]);
  });
});
