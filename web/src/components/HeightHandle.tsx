// The handle on the bottom edge of a canvas or ink block that sets its height. Dragging it with a
// mouse, a finger or a stylus shows the height as it goes and reports it once, on release, so a
// resize is one change and one saved version. Up and Down step it by 20 (Shift: 100), Home and
// End go to the bounds, and a double-click puts back the default.

import { Show, createSignal } from "solid-js";

/** A block's height: the default, and the bounds a drag or a key keeps it within. */
export interface HeightRange {
  initial: number;
  min: number;
  max: number;
}

export const HEIGHT_STEP = 20;
export const HEIGHT_STEP_BIG = 100;

export function clampTo(range: HeightRange, h: number): number {
  return Math.round(Math.min(range.max, Math.max(range.min, h)));
}

/** The height a key on the handle asks for, or null when the key does not resize. */
export function keyHeight(range: HeightRange, now: number, key: string, shift: boolean): number | null {
  const step = shift ? HEIGHT_STEP_BIG : HEIGHT_STEP;
  if (key === "ArrowDown") return clampTo(range, now + step);
  if (key === "ArrowUp") return clampTo(range, now - step);
  if (key === "Home") return range.min;
  if (key === "End") return range.max;
  return null;
}

/** The height a drag ends on: where it began, moved by `dy` screen pixels at `scale` pixels per unit. */
export function dragHeight(range: HeightRange, from: number, dy: number, scale = 1): number {
  return clampTo(range, from + dy / (scale > 0 ? scale : 1));
}

export function HeightHandle(props: {
  /** The accessible name, as "Canvas height". */
  label: string;
  /** The stored height, in the block's units. */
  height: number;
  range: HeightRange;
  /** Screen pixels per unit of height, read when a drag starts. Default 1. */
  scale?: () => number;
  /** The height while dragging, and null once the drag ends. */
  onPreview: (h: number | null) => void;
  /** The new height, once per drag, key press or reset, and only when it changed. */
  onChange: (h: number) => void;
  testid?: string;
  id?: string;
}) {
  const [shown, setShown] = createSignal<number | null>(null);
  let drag: { pointer: number; sy: number; from: number; scale: number } | null = null;

  const preview = (h: number | null) => {
    setShown(h);
    props.onPreview(h);
  };

  const set = (h: number) => {
    if (h !== props.height) props.onChange(h);
  };

  function onDown(e: PointerEvent) {
    if (e.button !== 0 || drag) return;
    e.preventDefault();
    e.stopPropagation();
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer has nothing to capture; its events still arrive here.
    }
    drag = { pointer: e.pointerId, sy: e.clientY, from: props.height, scale: props.scale?.() ?? 1 };
    preview(props.height);
  }

  function onMove(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    e.preventDefault();
    preview(dragHeight(props.range, d.from, e.clientY - d.sy, d.scale));
  }

  function onUp(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    drag = null;
    const h = e.type === "pointercancel" ? d.from : dragHeight(props.range, d.from, e.clientY - d.sy, d.scale);
    preview(null);
    set(h);
  }

  function onKey(e: KeyboardEvent) {
    const next = keyHeight(props.range, props.height, e.key, e.shiftKey);
    if (next === null) return;
    e.preventDefault();
    e.stopPropagation();
    set(next);
  }

  return (
    <div
      class="canvas-height-handle height-handle"
      classList={{ dragging: shown() !== null }}
      role="separator"
      aria-orientation="horizontal"
      aria-label={props.label}
      aria-valuemin={props.range.min}
      aria-valuemax={props.range.max}
      aria-valuenow={shown() ?? props.height}
      title={`Drag to set the height; Up and Down arrows resize it, double-click resets it to ${props.range.initial}`}
      tabindex={0}
      id={props.id}
      data-testid={props.testid}
      data-node-view-interactive=""
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onKeyDown={onKey}
      onDblClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        set(props.range.initial);
      }}
      // An editor's block node is draggable; resizing must not start a drag.
      onDragStart={(e) => e.preventDefault()}
    >
      <span class="canvas-height-grip" aria-hidden="true" />
      <Show when={shown()}>
        {(h) => (
          <span class="height-readout" data-testid="height-readout" aria-hidden="true">
            {h()} px
          </span>
        )}
      </Show>
    </div>
  );
}
