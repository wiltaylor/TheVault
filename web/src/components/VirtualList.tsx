import { type JSX, For, createMemo, createSignal, onCleanup, onMount } from "solid-js";

export interface VirtualListProps {
  /** Rows in the whole list, loaded or not. */
  count: number;
  /** Every row's height in pixels. */
  rowHeight: number;
  /** Rows rendered beyond each edge of the viewport. */
  overscan?: number;
  /** Told which rows are on screen whenever that changes, so the caller can load them. */
  onRange?: (start: number, end: number) => void;
  children: (index: number) => JSX.Element;
  class?: string;
  "data-testid"?: string;
  "aria-label"?: string;
}

/** The viewport height to assume before layout has measured one, as in tests without layout. */
const FALLBACK_HEIGHT = 800;

/**
 * A scrolling list that only renders the rows in view (D33), so a list of any length keeps a
 * few dozen rows in the DOM. Rows have one fixed height.
 */
export function VirtualList(props: VirtualListProps) {
  let el!: HTMLDivElement;
  const [scrollTop, setScrollTop] = createSignal(0);
  const [height, setHeight] = createSignal(FALLBACK_HEIGHT);

  const range = createMemo(() => {
    const overscan = props.overscan ?? 10;
    const first = Math.floor(scrollTop() / props.rowHeight);
    const visible = Math.ceil(height() / props.rowHeight);
    const start = Math.max(0, first - overscan);
    const end = Math.min(props.count, first + visible + overscan);
    return { start, end: Math.max(start, end) };
  });

  const indices = createMemo(() => {
    const { start, end } = range();
    const out: number[] = [];
    for (let i = start; i < end; i++) out.push(i);
    return out;
  });

  createMemo(() => {
    const { start, end } = range();
    props.onRange?.(start, end);
  });

  onMount(() => {
    const measure = () => setHeight(el.clientHeight || FALLBACK_HEIGHT);
    measure();
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(measure);
      observer.observe(el);
      onCleanup(() => observer.disconnect());
    }
  });

  return (
    <div
      ref={el}
      class={`virtual-list ${props.class ?? ""}`}
      role="list"
      aria-label={props["aria-label"]}
      data-testid={props["data-testid"]}
      onScroll={() => setScrollTop(el.scrollTop)}
    >
      <div class="virtual-list-body" style={{ height: `${props.count * props.rowHeight}px` }}>
        <For each={indices()}>
          {(i) => (
            <div
              class="virtual-list-row"
              role="listitem"
              style={{ top: `${i * props.rowHeight}px`, height: `${props.rowHeight}px` }}
            >
              {props.children(i)}
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
