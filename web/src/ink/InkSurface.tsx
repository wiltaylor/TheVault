// The one ink component (D18, D29): an ink content block and an ink field both draw through it.
// Pointer events carry the mouse, touch and stylus alike; a stylus's pressure is kept and a
// device without pressure stores 0.5. Erasing removes whole strokes, and undo steps back through
// this session's changes.

import { For, Show, createSignal } from "solid-js";
import type { Block } from "../api";
import {
  BACKGROUNDS,
  type Background,
  type Ink,
  type Stroke,
  type Tool,
  hits,
  inkBlock,
  inkOf,
  paintOrder,
  pointerPressure,
  storedPoint,
  strokePath,
} from "./ink";

type Mode = Tool | "eraser";

const MODES: { mode: Mode; label: string }[] = [
  { mode: "pen", label: "Pen" },
  { mode: "highlighter", label: "Highlighter" },
  { mode: "eraser", label: "Eraser" },
];

const SIZES = [2, 4, 8, 16];

/** The eraser's reach around the pointer, in ink units. */
const ERASER_RADIUS = 6;

/** A stylus's eraser end, or its barrel button held as one (Pointer Events, `buttons` bit 32). */
const ERASER_BUTTON = 32;

let surfaces = 0;

/**
 * An ink drawing. Without `onChange` it only shows the drawing; with it the drawing is edited
 * and every finished stroke, erase or undo reports the whole new block.
 */
export function InkSurface(props: { block: Block | undefined; label: string; onChange?: (block: Block) => void }) {
  const id = `ink-${++surfaces}`;
  const ink = () => inkOf(props.block);
  const [mode, setMode] = createSignal<Mode>("pen");
  const [colour, setColour] = createSignal("#1f2937");
  const [size, setSize] = createSignal(4);
  // The stroke being drawn, or the strokes left while an erase gesture runs.
  const [live, setLive] = createSignal<Stroke | null>(null);
  const [erased, setErased] = createSignal<Stroke[] | null>(null);
  const [history, setHistory] = createSignal<Block[]>([]);
  let svg!: SVGSVGElement;
  let pointer: number | null = null;

  const strokes = () => erased() ?? ink().strokes;

  function commit(next: Ink) {
    const before = props.block ?? inkBlock(ink());
    setHistory([...history(), before]);
    props.onChange?.(inkBlock(next, props.block));
  }

  function undo() {
    const h = history();
    const prev = h.at(-1);
    if (!prev) return;
    setHistory(h.slice(0, -1));
    props.onChange?.(prev);
  }

  /** The pointer's position in ink units. */
  function at(e: PointerEvent): [number, number] {
    const r = svg.getBoundingClientRect();
    const { width, height } = ink();
    return [((e.clientX - r.left) * width) / (r.width || width), ((e.clientY - r.top) * height) / (r.height || height)];
  }

  function erase(e: PointerEvent) {
    const [x, y] = at(e);
    const left = erased();
    if (!left) return;
    const kept = left.filter((s) => !hits(s, x, y, ERASER_RADIUS));
    if (kept.length !== left.length) setErased(kept);
  }

  function extend(e: PointerEvent) {
    const s = live();
    if (!s) return;
    const events = e.getCoalescedEvents?.() ?? [];
    const points = [...s.points];
    for (const ev of events.length ? events : [e]) {
      const [x, y] = at(ev);
      points.push(...storedPoint(x, y, pointerPressure(ev)));
    }
    setLive({ ...s, points });
  }

  function onPointerDown(e: PointerEvent) {
    if (!props.onChange || pointer !== null) return;
    const eraserButton = (e.buttons & ERASER_BUTTON) !== 0;
    if (e.button !== 0 && !eraserButton) return;
    e.preventDefault();
    pointer = e.pointerId;
    try {
      svg.setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer has nothing to capture; its events still arrive here.
    }
    svg.focus({ preventScroll: true });
    if (mode() === "eraser" || eraserButton) {
      setErased(ink().strokes);
      erase(e);
    } else {
      const [x, y] = at(e);
      setLive({ tool: mode() as Tool, colour: colour(), size: size(), points: storedPoint(x, y, pointerPressure(e)) });
    }
  }

  function onPointerMove(e: PointerEvent) {
    if (e.pointerId !== pointer) return;
    e.preventDefault();
    if (erased()) erase(e);
    else extend(e);
  }

  function onPointerUp(e: PointerEvent) {
    if (e.pointerId !== pointer) return;
    pointer = null;
    const s = live();
    const left = erased();
    setLive(null);
    setErased(null);
    if (s) commit({ ...ink(), strokes: [...ink().strokes, s] });
    else if (left && left.length !== ink().strokes.length) commit({ ...ink(), strokes: left });
  }

  function onKeyDown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
      e.preventDefault();
      undo();
    }
  }

  return (
    <div class="ink" data-testid="ink">
      <Show when={props.onChange}>
        <div class="ink-toolbar" role="toolbar" aria-label={`${props.label} tools`}>
          <For each={MODES}>
            {(m) => (
              <button type="button" aria-pressed={mode() === m.mode} onClick={() => setMode(m.mode)}>
                {m.label}
              </button>
            )}
          </For>
          <input
            type="color"
            aria-label="Ink colour"
            value={colour()}
            onInput={(e) => setColour(e.currentTarget.value.toLowerCase())}
          />
          <select aria-label="Stroke size" value={size()} onChange={(e) => setSize(Number(e.currentTarget.value))}>
            <For each={SIZES}>{(s) => <option value={s}>{s}</option>}</For>
          </select>
          <select
            aria-label="Background"
            value={ink().background}
            onChange={(e) => commit({ ...ink(), background: e.currentTarget.value as Background })}
          >
            <For each={BACKGROUNDS}>{(b) => <option value={b}>{b}</option>}</For>
          </select>
          <button type="button" disabled={history().length === 0} onClick={undo}>
            Undo
          </button>
        </div>
      </Show>
      <svg
        ref={svg}
        class="ink-surface"
        classList={{ editing: !!props.onChange, [`ink-${mode()}`]: !!props.onChange }}
        role="img"
        aria-label={props.label}
        tabindex={props.onChange ? 0 : undefined}
        data-node-view-interactive={props.onChange ? "" : undefined}
        width={ink().width}
        height={ink().height}
        viewBox={`0 0 ${ink().width} ${ink().height}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
        // The editor's block node is draggable; drawing on it must not start a drag.
        onDragStart={(e) => props.onChange && e.preventDefault()}
      >
        <defs>
          <pattern id={`${id}-lines`} width="32" height="32" patternUnits="userSpaceOnUse">
            <path d="M0 31.5H32" class="ink-guide" />
          </pattern>
          <pattern id={`${id}-grid`} width="32" height="32" patternUnits="userSpaceOnUse">
            <path d="M0 31.5H32M31.5 0V32" class="ink-guide" />
          </pattern>
          <pattern id={`${id}-dots`} width="32" height="32" patternUnits="userSpaceOnUse">
            <circle cx="16" cy="16" r="1.25" class="ink-dot" />
          </pattern>
        </defs>
        <rect
          class="ink-paper"
          width="100%"
          height="100%"
          fill={ink().background === "blank" ? "transparent" : `url(#${id}-${ink().background})`}
        />
        <For each={paintOrder(strokes())}>
          {(s) => (
            <path
              class={`ink-stroke ink-${s.stroke.tool}`}
              data-index={s.index}
              d={strokePath(s.stroke)}
              fill={s.stroke.colour}
            />
          )}
        </For>
        <Show when={live()}>
          {(s) => <path class={`ink-stroke ink-live ink-${s().tool}`} d={strokePath(s(), false)} fill={s().colour} />}
        </Show>
      </svg>
    </div>
  );
}
