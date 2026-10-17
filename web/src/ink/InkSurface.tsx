// The one ink component (D18, D29): an ink content block and an ink field both draw through it.
// The block is a view onto an endless plane. A stylus draws (its pressure is kept) and erases
// with its eraser end; the mouse and fingers pan, two fingers pinch to zoom, and the wheel pans
// (Ctrl or Cmd, or a trackpad pinch, zooms). "Draw with mouse and touch" lets the mouse and one
// finger draw instead; the choice is kept per device. Erasing removes whole strokes, and undo
// steps back through this session's changes. The view is saved with the drawing, a moment after
// it stops moving, and is not an undo step.

import { For, type JSX, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import type { Block } from "../api";
import { HeightHandle } from "../components/HeightHandle";
import {
  BACKGROUNDS,
  INK_HEIGHT,
  NEW_WIDTH,
  TOOL_STYLES,
  PAPER,
  type Background,
  type Ink,
  type InkTheme,
  type InkView,
  type Mode,
  type Route,
  type Stroke,
  type Tool,
  type ToolStyle,
  cachedPath,
  defaultView,
  fitView,
  hits,
  inkBlock,
  inkOf,
  panBy,
  patternStep,
  pinchView,
  pointerPressure,
  renderColour,
  routePress,
  storedPoint,
  storedView,
  strokePath,
  toInk,
  visibleStrokes,
  zoomAbout,
} from "./ink";
import { EraserIcon, FitIcon, HighlighterIcon, PaperIcon, PenIcon, SizeIcon, TouchDrawIcon, UndoIcon } from "./InkIcons";

const MODES: { mode: Mode; label: string; Icon: () => JSX.Element }[] = [
  { mode: "pen", label: "Pen", Icon: PenIcon },
  { mode: "highlighter", label: "Highlighter", Icon: HighlighterIcon },
  { mode: "eraser", label: "Eraser", Icon: EraserIcon },
];

type Popover = "colour" | "size" | "paper";

const BACKGROUND_NAMES: Record<Background, string> = { blank: "Blank", lines: "Lines", grid: "Grid", dots: "Dots" };

/** The eraser's reach around the pointer, in screen pixels. */
const ERASER_RADIUS = 6;

/** Wait this long after the view stops moving before saving it. */
export const VIEW_SAVE_MS = 400;

/** One button or key step of zoom, and of keyboard panning in screen pixels. */
const ZOOM_STEP = 1.25;
const PAN_STEP = 48;

// Whether the mouse and one finger draw rather than pan: per device, so in local storage, and
// shared by every drawing on the page.
const TOUCH_DRAWS_KEY = "thevault.ink-touch-draws";

function readTouchDraws(): boolean {
  try {
    return localStorage.getItem(TOUCH_DRAWS_KEY) === "on";
  } catch {
    return false;
  }
}

const [touchDraws, setTouchDrawsSignal] = createSignal(readTouchDraws());

function setTouchDraws(on: boolean) {
  setTouchDrawsSignal(on);
  try {
    localStorage.setItem(TOUCH_DRAWS_KEY, on ? "on" : "off");
  } catch {
    // Without storage the choice lasts until the page reloads.
  }
}

/** Read the stored choice again: for tests, which change local storage between surfaces. */
export function reloadTouchDraws() {
  setTouchDrawsSignal(readTouchDraws());
}

// The page's theme, as `data-theme` on <html> says: strokes, guides and swatches draw in its
// colours, and draw again when it changes, without a reload.
const themeOfPage = (): InkTheme => (document.documentElement.dataset.theme === "dark" ? "dark" : "light");
const [pageTheme, setPageTheme] = createSignal<InkTheme>(themeOfPage());
if (typeof MutationObserver !== "undefined") {
  new MutationObserver(() => setPageTheme(themeOfPage())).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
}

type Pt = { x: number; y: number };

type Gesture =
  | { kind: "draw" | "erase"; pointer: number; type: string }
  | { kind: "pan"; pointer: number; type: string; start: Pt; from: InkView }
  | { kind: "pinch"; a: number; b: number; a0: Pt; b0: Pt; from: InkView };

let surfaces = 0;

/**
 * An ink drawing. Without `onChange` it only shows the drawing at its saved view; with it the
 * drawing is edited and every finished stroke, erase, undo or settled view reports the whole new
 * block.
 */
export function InkSurface(props: { block: Block | undefined; label: string; onChange?: (block: Block) => void }) {
  const id = `ink-${++surfaces}`;
  const ink = createMemo(() => inkOf(props.block));
  const editing = () => !!props.onChange;
  const [mode, setMode] = createSignal<Mode>("pen");
  // The tool the colour and size controls set: the one in use, or the last one used before the eraser.
  const [drawTool, setDrawTool] = createSignal<Tool>("pen");
  const [styles, setStyles] = createSignal<Record<Tool, ToolStyle>>({
    pen: { ...TOOL_STYLES.pen.initial },
    highlighter: { ...TOOL_STYLES.highlighter.initial },
  });
  const style = () => styles()[drawTool()];
  const setStyle = (s: Partial<ToolStyle>) => setStyles({ ...styles(), [drawTool()]: { ...style(), ...s } });
  // The stroke being drawn, or the strokes left while an erase gesture runs.
  const [live, setLive] = createSignal<Stroke | null>(null);
  const [erased, setErased] = createSignal<Stroke[] | null>(null);
  const [history, setHistory] = createSignal<Block[]>([]);
  let svg!: SVGSVGElement;

  // The surface's size on screen, without its border. jsdom lays nothing out, and there the view
  // is as wide as the drawing's page, or the default page.
  const [box, setBox] = createSignal({ width: 0, height: 0 });
  // The height while the bottom handle is dragged. Strokes keep their coordinates: a taller view
  // shows more of the plane, a shorter one less.
  const [heightPreview, setHeightPreview] = createSignal<number | null>(null);
  const viewHeight = () => heightPreview() ?? ink().height;
  const viewWidth = () => box().width || ink().width || NEW_WIDTH;

  // The view as panned and zoomed here, until it is saved and comes back in the block.
  const [moved, setMoved] = createSignal<InkView | null>(null);
  const view = (): InkView => moved() ?? ink().view ?? defaultView(ink(), box().width);
  const [panning, setPanning] = createSignal(false);
  const [penHover, setPenHover] = createSignal(false);
  let gesture: Gesture | null = null;
  let viewTimer: ReturnType<typeof setTimeout> | undefined;
  // The fingers on the surface that count, in screen pixels from its top-left, and the stylus.
  const fingers = new Map<number, Pt>();
  let penDown: number | null = null;

  // A block that changes from outside, as after a reload or a save's reply, brings its view,
  // unless the view here is moving or waiting to be saved.
  createEffect(
    on(
      () => ink().view,
      () => {
        if (!gesture && viewTimer === undefined) setMoved(null);
      },
      { defer: true },
    ),
  );

  /** `next` with the view as it is here, when it has moved. */
  const withView = (next: Ink): Ink => {
    const v = moved();
    return v ? { ...next, view: v } : next;
  };

  function commit(next: Ink) {
    const before = props.block ?? inkBlock(ink());
    setHistory([...history(), before]);
    clearTimeout(viewTimer);
    viewTimer = undefined;
    props.onChange?.(inkBlock(withView(next), props.block));
  }

  function undo() {
    const h = history();
    const prev = h.at(-1);
    if (!prev) return;
    setHistory(h.slice(0, -1));
    // Undo takes back the drawing, not the view: it stays where it is.
    const v = moved() ?? ink().view;
    props.onChange?.(v ? inkBlock({ ...inkOf(prev), view: v }, prev) : prev);
  }

  /** Save the view once it has been still for a moment. Saving it is not an undo step. */
  function saveViewSoon() {
    if (!editing()) return;
    clearTimeout(viewTimer);
    viewTimer = setTimeout(saveView, VIEW_SAVE_MS);
  }

  function saveView() {
    clearTimeout(viewTimer);
    viewTimer = undefined;
    const v = moved();
    if (!v || !props.onChange) return;
    const was = ink().view;
    const next = storedView(v);
    if (was && was.x === next.x && was.y === next.y && was.zoom === next.zoom) return;
    props.onChange(inkBlock({ ...ink(), view: v }, props.block));
  }

  function setView(v: InkView) {
    setMoved(v);
  }

  /** A pointer's position in screen pixels from the surface's top-left, inside its border. */
  function local(e: { clientX: number; clientY: number }): Pt {
    const r = svg.getBoundingClientRect();
    return { x: e.clientX - r.left - svg.clientLeft, y: e.clientY - r.top - svg.clientTop };
  }

  /** The pointer's position in ink units. */
  function at(e: { clientX: number; clientY: number }): [number, number] {
    const p = local(e);
    return toInk(view(), p.x, p.y);
  }

  function erase(e: PointerEvent) {
    const [x, y] = at(e);
    const left = erased();
    if (!left) return;
    const kept = left.filter((s) => !hits(s, x, y, ERASER_RADIUS / view().zoom));
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

  function capture(e: PointerEvent) {
    try {
      svg.setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer has nothing to capture; its events still arrive here.
    }
  }

  /** Drop the gesture under way without keeping what it drew or erased. */
  function abandon() {
    const g = gesture;
    gesture = null;
    setLive(null);
    setErased(null);
    setPanning(false);
    if (g && (g.kind === "pan" || g.kind === "pinch")) saveViewSoon();
  }

  function start(route: Route, e: PointerEvent) {
    e.preventDefault();
    capture(e);
    svg.focus({ preventScroll: true });
    if (route === "pan") {
      gesture = { kind: "pan", pointer: e.pointerId, type: e.pointerType, start: local(e), from: view() };
      setPanning(true);
      clearTimeout(viewTimer);
    } else if (route === "erase") {
      gesture = { kind: "erase", pointer: e.pointerId, type: e.pointerType };
      setErased(ink().strokes);
      erase(e);
    } else if (route === "draw") {
      gesture = { kind: "draw", pointer: e.pointerId, type: e.pointerType };
      const tool = mode() as Tool;
      const s = styles()[tool];
      const [x, y] = at(e);
      setLive({ tool, colour: s.colour, size: s.size, points: storedPoint(x, y, pointerPressure(e)) });
    }
  }

  function onPointerDown(e: PointerEvent) {
    if (!editing()) return;
    const route = routePress(e, { mode: mode(), touchDraws: touchDraws(), penDown: penDown !== null, fingers: fingers.size });
    if (route === "ignore") {
      if (e.pointerType === "touch") e.preventDefault();
      return;
    }
    if (e.pointerType === "pen") {
      if (penDown !== null) return;
      // A palm that landed first gives way to the stylus.
      if (gesture) abandon();
      fingers.clear();
      penDown = e.pointerId;
      start(route, e);
      return;
    }
    if (e.pointerType === "touch") {
      fingers.set(e.pointerId, local(e));
      if (route === "pinch") {
        // The second finger: what the first began is dropped, and the two pan and zoom.
        if (gesture && gesture.kind !== "pan") abandon();
        e.preventDefault();
        capture(e);
        const [a, b] = [...fingers.keys()] as [number, number];
        gesture = { kind: "pinch", a, b, a0: fingers.get(a)!, b0: fingers.get(b)!, from: view() };
        setPanning(true);
        clearTimeout(viewTimer);
        return;
      }
      if (gesture) return;
      start(route, e);
      return;
    }
    if (gesture) return;
    start(route, e);
  }

  function onPointerMove(e: PointerEvent) {
    if (e.pointerType === "pen" || e.pointerType === "mouse") setPenHover(e.pointerType === "pen");
    if (fingers.has(e.pointerId)) fingers.set(e.pointerId, local(e));
    const g = gesture;
    if (!g) return;
    if (g.kind === "pinch") {
      if (e.pointerId !== g.a && e.pointerId !== g.b) return;
      e.preventDefault();
      const a1 = fingers.get(g.a);
      const b1 = fingers.get(g.b);
      if (a1 && b1) setView(pinchView(g.from, g.a0, g.b0, a1, b1));
      return;
    }
    if (e.pointerId !== g.pointer) return;
    e.preventDefault();
    if (g.kind === "pan") {
      const p = local(e);
      setView(panBy(g.from, p.x - g.start.x, p.y - g.start.y));
    } else if (g.kind === "erase") erase(e);
    else extend(e);
  }

  function onPointerUp(e: PointerEvent) {
    if (e.pointerId === penDown) penDown = null;
    fingers.delete(e.pointerId);
    const g = gesture;
    if (!g) return;
    if (g.kind === "pinch") {
      if (e.pointerId !== g.a && e.pointerId !== g.b) return;
      // One finger left: it carries on panning from where it is.
      const rest = e.pointerId === g.a ? g.b : g.a;
      const p = fingers.get(rest);
      gesture = p ? { kind: "pan", pointer: rest, type: "touch", start: p, from: view() } : null;
      if (!p) setPanning(false);
      saveViewSoon();
      return;
    }
    if (e.pointerId !== g.pointer) return;
    gesture = null;
    if (g.kind === "pan") {
      setPanning(false);
      saveViewSoon();
      return;
    }
    const s = live();
    const left = erased();
    setLive(null);
    setErased(null);
    if (s) commit({ ...ink(), strokes: [...ink().strokes, s] });
    else if (left && left.length !== ink().strokes.length) commit({ ...ink(), strokes: left });
  }

  /** Zoom by `factor` about the middle of the view, or a screen point in it. */
  function zoomBy(factor: number, p?: Pt) {
    setView(zoomAbout(view(), factor, p?.x ?? viewWidth() / 2, p?.y ?? viewHeight() / 2));
    saveViewSoon();
  }

  function fit() {
    setView(fitView(ink().strokes, viewWidth(), viewHeight()));
    saveViewSoon();
  }

  function onKeyDown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && !e.shiftKey && e.key.toLowerCase() === "z") {
      e.preventDefault();
      undo();
      return;
    }
    if (mod || e.altKey) return;
    const step = e.shiftKey ? PAN_STEP * 4 : PAN_STEP;
    const pans: Record<string, [number, number]> = {
      ArrowLeft: [step, 0],
      ArrowRight: [-step, 0],
      ArrowUp: [0, step],
      ArrowDown: [0, -step],
    };
    const pan = pans[e.key];
    if (pan) {
      setView(panBy(view(), pan[0], pan[1]));
      saveViewSoon();
    } else if (e.key === "+" || e.key === "=") zoomBy(ZOOM_STEP);
    else if (e.key === "-") zoomBy(1 / ZOOM_STEP);
    else if (e.key === "0") zoomBy(1 / view().zoom);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  onMount(() => {
    const measure = () => setBox({ width: svg.clientWidth, height: svg.clientHeight });
    const sizer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    sizer?.observe(svg);
    measure();
    // Not a Solid handler: the wheel must be able to cancel the page's own scroll.
    const onWheel = (e: WheelEvent) => {
      if (!editing()) return;
      e.preventDefault();
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? viewHeight() : 1;
      if (e.ctrlKey || e.metaKey) {
        // A trackpad pinch arrives as a wheel with Ctrl held, in small steps.
        zoomBy(Math.exp(-e.deltaY * unit * 0.0025), local(e));
        return;
      }
      let dx = e.deltaX * unit;
      let dy = e.deltaY * unit;
      if (e.shiftKey && dx === 0) [dx, dy] = [dy, 0];
      setView(panBy(view(), -dx, -dy));
      saveViewSoon();
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    onCleanup(() => {
      sizer?.disconnect();
      svg.removeEventListener("wheel", onWheel);
      clearTimeout(viewTimer);
    });
  });

  // ---- what is drawn ------------------------------------------------------------------------

  const strokes = () => erased() ?? ink().strokes;
  const visible = createMemo(() => visibleStrokes(strokes(), view(), viewWidth(), viewHeight()));
  const highlighters = () => visible().filter((s) => s.tool === "highlighter");
  const pens = () => visible().filter((s) => s.tool === "pen");
  const step = () => patternStep(view().zoom);
  const paper = () => PAPER[pageTheme()];
  const fillOf = (s: Stroke) => renderColour(s.colour, s.tool, pageTheme());
  const zoomPercent = () => Math.round(view().zoom * 100);

  // ---- the controls -------------------------------------------------------------------------

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

  const colours = () => TOOL_STYLES[drawTool()].colours;
  const colourName = (colour: string) => colours().find((c) => c.value === colour)?.name ?? colour;
  /** A colour as a stroke of the tool the controls set draws it in this theme. */
  const shown = (colour: string) => renderColour(colour, drawTool(), pageTheme());

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
                    onClick={() => {
                      setMode(m.mode);
                      if (m.mode !== "eraser") setDrawTool(m.mode);
                    }}
                  >
                    <m.Icon />
                  </button>
                )}
              </For>
              <span class="ink-divider" aria-hidden="true" />
              <span class="ink-popover-wrap">
                <Trigger popover="colour" label="Colour" title={`Colour: ${colourName(style().colour)}`}>
                  <span class="ink-swatch" style={{ background: shown(style().colour) }} />
                </Trigger>
                <Show when={open() === "colour"}>
                  <div class="ink-popover ink-palette" role="group" aria-label="Ink colours" onKeyDown={onPopoverKey} ref={focusChosen}>
                    <For each={colours()}>
                      {(c) => (
                        <button
                          type="button"
                          class="ink-palette-item"
                          aria-label={c.name}
                          title={c.name}
                          aria-pressed={style().colour === c.value}
                          data-colour={c.value}
                          onClick={() => {
                            setStyle({ colour: c.value });
                            close(true);
                          }}
                        >
                          <span class="ink-swatch" style={{ background: shown(c.value) }} />
                        </button>
                      )}
                    </For>
                    <label class="ink-custom-colour" title="Any colour">
                      <input
                        type="color"
                        aria-label="Ink colour"
                        value={shown(style().colour)}
                        onInput={(e) => setStyle({ colour: e.currentTarget.value.toLowerCase() })}
                      />
                      <span>Custom</span>
                    </label>
                  </div>
                </Show>
              </span>
              <span class="ink-popover-wrap">
                <Trigger popover="size" label="Stroke size" title={`Stroke size: ${style().size}`}>
                  <SizeIcon size={style().size} />
                </Trigger>
                <Show when={open() === "size"}>
                  <div class="ink-popover ink-menu" role="group" aria-label="Stroke sizes" onKeyDown={onPopoverKey} ref={focusChosen}>
                    <For each={TOOL_STYLES[drawTool()].sizes}>
                      {(s) => (
                        <button
                          type="button"
                          class="ink-menu-item"
                          aria-label={`Size ${s}`}
                          title={`Stroke size ${s}`}
                          aria-pressed={style().size === s}
                          onClick={() => {
                            setStyle({ size: s });
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
              <span class="ink-divider" aria-hidden="true" />
              <button
                type="button"
                class="ink-action"
                aria-label="Draw with mouse and touch"
                title={
                  touchDraws()
                    ? "Draw with mouse and touch: on. The mouse and one finger draw; two fingers pan and zoom"
                    : "Draw with mouse and touch: off. The mouse and fingers pan; a stylus draws"
                }
                aria-pressed={touchDraws()}
                data-testid="ink-touch-draws"
                onClick={() => setTouchDraws(!touchDraws())}
              >
                <TouchDrawIcon />
              </button>
            </div>
            <div class="ink-cluster" role="toolbar" aria-label={`${props.label} actions`}>
              <button
                type="button"
                class="ink-action ink-zoom"
                aria-label={`Zoom ${zoomPercent()}%, reset to 100%`}
                title={`Zoom ${zoomPercent()}%. Click for 100%; Ctrl+wheel or a pinch zooms`}
                data-testid="ink-zoom"
                onClick={() => zoomBy(1 / view().zoom)}
              >
                {zoomPercent()}%
              </button>
              <button type="button" class="ink-action" aria-label="Fit drawing" title="Fit drawing: show every stroke" onClick={fit}>
                <FitIcon />
              </button>
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
          classList={{
            editing: editing(),
            [`mode-${mode()}`]: editing(),
            pans: editing() && !touchDraws() && !penHover(),
            panning: panning(),
          }}
          role="img"
          aria-label={props.label}
          data-theme={pageTheme()}
          style={{ background: paper().paper }}
          tabindex={editing() ? 0 : undefined}
          data-node-view-interactive={editing() ? "" : undefined}
          data-zoom={view().zoom}
          width="100%"
          // The box is border-box: its 1px border on each side, around the view's height.
          height={viewHeight() + 2}
          viewBox={`${view().x} ${view().y} ${viewWidth() / view().zoom} ${viewHeight() / view().zoom}`}
          preserveAspectRatio="xMinYMin meet"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onPointerLeave={(e) => e.pointerType === "pen" && setPenHover(false)}
          onKeyDown={onKeyDown}
          onBlur={() => viewTimer !== undefined && saveView()}
          // The editor's block node is draggable; drawing on it must not start a drag.
          onDragStart={(e) => editing() && e.preventDefault()}
        >
          <defs>
            <pattern id={`${id}-lines`} width={step()} height={step()} patternUnits="userSpaceOnUse">
              <path d={`M0 ${step() / 2}H${step()}`} class="ink-guide" stroke={paper().guide} vector-effect="non-scaling-stroke" />
            </pattern>
            <pattern id={`${id}-grid`} width={step()} height={step()} patternUnits="userSpaceOnUse">
              <path
                d={`M0 ${step() / 2}H${step()}M${step() / 2} 0V${step()}`}
                class="ink-guide"
                stroke={paper().guide}
                vector-effect="non-scaling-stroke"
              />
            </pattern>
            <pattern id={`${id}-dots`} width={step()} height={step()} patternUnits="userSpaceOnUse">
              <circle cx={step() / 2} cy={step() / 2} r={1.25 / view().zoom} class="ink-dot" fill={paper().dot} />
            </pattern>
          </defs>
          <rect
            class="ink-paper"
            x={view().x}
            y={view().y}
            width={viewWidth() / view().zoom}
            height={viewHeight() / view().zoom}
            fill={ink().background === "blank" ? "transparent" : `url(#${id}-${ink().background})`}
          />
          <For each={highlighters()}>
            {(s) => <path class="ink-stroke ink-highlighter" d={cachedPath(s)} fill={fillOf(s)} />}
          </For>
          <For each={pens()}>{(s) => <path class="ink-stroke ink-pen" d={cachedPath(s)} fill={fillOf(s)} />}</For>
          <Show when={live()}>
            {(s) => <path class={`ink-stroke ink-live ink-${s().tool}`} d={strokePath(s(), false)} fill={fillOf(s())} />}
          </Show>
        </svg>
        <Show when={props.onChange}>
          <HeightHandle
            label={`${props.label} height`}
            height={ink().height}
            range={INK_HEIGHT}
            onPreview={setHeightPreview}
            onChange={(height) => commit({ ...ink(), height })}
            testid="ink-height-handle"
          />
        </Show>
      </div>
    </div>
  );
}
