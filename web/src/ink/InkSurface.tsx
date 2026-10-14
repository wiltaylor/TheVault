// The one ink component (D18, D29): an ink content block and an ink field both draw through it.
// Pointer events carry the mouse, touch and stylus alike; a stylus's pressure is kept and a
// device without pressure stores 0.5. Erasing removes whole strokes, and undo steps back through
// this session's changes.

import { For, type JSX, Show, createEffect, createSignal, onCleanup } from "solid-js";
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
import { EraserIcon, HighlighterIcon, PaperIcon, PenIcon, SizeIcon, UndoIcon } from "./InkIcons";

type Mode = Tool | "eraser";

const MODES: { mode: Mode; label: string; Icon: () => JSX.Element }[] = [
  { mode: "pen", label: "Pen", Icon: PenIcon },
  { mode: "highlighter", label: "Highlighter", Icon: HighlighterIcon },
  { mode: "eraser", label: "Eraser", Icon: EraserIcon },
];

type Popover = "colour" | "size" | "paper";

/**
 * The ink colours offered as swatches: the default ink and the inline palette's hues (OQ-S4).
 * Strokes store the hex, and the paper is light in both themes, so the hues are the light ones.
 */
const COLOURS: { name: string; hex: string }[] = [
  { name: "Ink", hex: "#1f2937" },
  { name: "Gray", hex: "#6b7280" },
  { name: "Brown", hex: "#92400e" },
  { name: "Orange", hex: "#ea580c" },
  { name: "Yellow", hex: "#ca8a04" },
  { name: "Green", hex: "#16a34a" },
  { name: "Blue", hex: "#2563eb" },
  { name: "Purple", hex: "#9333ea" },
  { name: "Pink", hex: "#db2777" },
  { name: "Red", hex: "#dc2626" },
];

const colourName = (hex: string) => COLOURS.find((c) => c.hex === hex)?.name ?? hex;

const BACKGROUND_NAMES: Record<Background, string> = { blank: "Blank", lines: "Lines", grid: "Grid", dots: "Dots" };

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

  // The open popover of the controls, if any, closes on Escape or a press anywhere else.
  const [open, setOpen] = createSignal<Popover | null>(null);
  let controls: HTMLDivElement | undefined;
  const triggers: Partial<Record<Popover, HTMLButtonElement>> = {};

  function toggle(p: Popover) {
    setOpen(open() === p ? null : p);
  }

  function close(refocus: boolean) {
    const p = open();
    setOpen(null);
    if (refocus && p) triggers[p]?.focus({ preventScroll: true });
  }

  createEffect(() => {
    if (!open()) return;
    const outside = (e: PointerEvent) => {
      if (!(e.target instanceof Node && controls?.contains(e.target))) close(false);
    };
    document.addEventListener("pointerdown", outside, true);
    onCleanup(() => document.removeEventListener("pointerdown", outside, true));
  });

  function onPopoverKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
    }
  }

  /** Focus a popover's chosen item, or its first, once it shows. */
  const focusChosen = (pop: HTMLElement) =>
    queueMicrotask(() =>
      (pop.querySelector<HTMLElement>("[aria-pressed='true']") ?? pop.querySelector<HTMLElement>("button, input"))?.focus({
        preventScroll: true,
      }),
    );

  function Trigger(p: { popover: Popover; label: string; title: string; children: JSX.Element }) {
    return (
      <button
        type="button"
        class="ink-action"
        aria-label={p.label}
        title={p.title}
        aria-haspopup="true"
        aria-expanded={open() === p.popover}
        ref={(el) => (triggers[p.popover] = el)}
        onClick={() => toggle(p.popover)}
      >
        {p.children}
      </button>
    );
  }

  return (
    <div class="ink" data-testid="ink">
      <div class="ink-stage">
      <Show when={props.onChange}>
        <div class="ink-controls" ref={controls} data-node-view-interactive="" data-testid="ink-controls">
          <div class="ink-cluster" role="toolbar" aria-label={`${props.label} tools`}>
            <For each={MODES}>
              {(m) => (
                <button
                  type="button"
                  class="ink-action"
                  aria-label={m.label}
                  title={m.label}
                  aria-pressed={mode() === m.mode}
                  onClick={() => setMode(m.mode)}
                >
                  <m.Icon />
                </button>
              )}
            </For>
            <span class="ink-divider" aria-hidden="true" />
            <span class="ink-popover-wrap">
              <Trigger popover="colour" label="Colour" title={`Colour: ${colourName(colour())}`}>
                <span class="ink-swatch" style={{ background: colour() }} />
              </Trigger>
              <Show when={open() === "colour"}>
                <div class="ink-popover ink-palette" role="group" aria-label="Ink colours" onKeyDown={onPopoverKey} ref={focusChosen}>
                  <For each={COLOURS}>
                    {(c) => (
                      <button
                        type="button"
                        class="ink-palette-item"
                        aria-label={c.name}
                        title={c.name}
                        aria-pressed={colour() === c.hex}
                        data-colour={c.hex}
                        onClick={() => {
                          setColour(c.hex);
                          close(true);
                        }}
                      >
                        <span class="ink-swatch" style={{ background: c.hex }} />
                      </button>
                    )}
                  </For>
                  <label class="ink-custom-colour" title="Any colour">
                    <input
                      type="color"
                      aria-label="Ink colour"
                      value={colour()}
                      onInput={(e) => setColour(e.currentTarget.value.toLowerCase())}
                    />
                    <span>Custom</span>
                  </label>
                </div>
              </Show>
            </span>
            <span class="ink-popover-wrap">
              <Trigger popover="size" label="Stroke size" title={`Stroke size: ${size()}`}>
                <SizeIcon size={size()} />
              </Trigger>
              <Show when={open() === "size"}>
                <div class="ink-popover ink-menu" role="group" aria-label="Stroke sizes" onKeyDown={onPopoverKey} ref={focusChosen}>
                  <For each={SIZES}>
                    {(s) => (
                      <button
                        type="button"
                        class="ink-menu-item"
                        aria-label={`Size ${s}`}
                        title={`Stroke size ${s}`}
                        aria-pressed={size() === s}
                        onClick={() => {
                          setSize(s);
                          close(true);
                        }}
                      >
                        <SizeIcon size={s} />
                        <span>{s}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </span>
          </div>
          <div class="ink-cluster" role="toolbar" aria-label={`${props.label} actions`}>
            <span class="ink-popover-wrap">
              <Trigger popover="paper" label="Background" title={`Background: ${BACKGROUND_NAMES[ink().background]}`}>
                <PaperIcon background={ink().background} />
              </Trigger>
              <Show when={open() === "paper"}>
                <div class="ink-popover ink-menu end" role="group" aria-label="Backgrounds" onKeyDown={onPopoverKey} ref={focusChosen}>
                  <For each={BACKGROUNDS}>
                    {(b) => (
                      <button
                        type="button"
                        class="ink-menu-item"
                        aria-pressed={ink().background === b}
                        onClick={() => {
                          if (ink().background !== b) commit({ ...ink(), background: b });
                          close(true);
                        }}
                      >
                        <PaperIcon background={b} />
                        <span>{BACKGROUND_NAMES[b]}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </span>
            <button
              type="button"
              class="ink-action"
              aria-label="Undo"
              title="Undo (Ctrl+Z)"
              disabled={history().length === 0}
              onClick={undo}
            >
              <UndoIcon />
            </button>
          </div>
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
    </div>
  );
}
