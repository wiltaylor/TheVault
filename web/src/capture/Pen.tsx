// The capture dialog's pen (capture K2): a drawing surface over the dialog, the app's own ink
// drawing on its endless plane, with Done and Discard. Done hands over the drawing as an `ink`
// content block; the dialog shows each one as a small preview until the capture is saved.

import { For, createSignal } from "solid-js";
import type { Block } from "../api";
import { InkSurface } from "../ink/InkSurface";
import { INK_HEIGHT, NEW_WIDTH, drawingBounds, fitView, inkBlock, inkOf, paintOrder, cachedPath, renderColour } from "../ink/ink";

/** A new, empty drawing. */
function emptyDrawing(): Block {
  return inkBlock({ height: INK_HEIGHT.initial, background: "blank", view: null, strokes: [] });
}

/**
 * The drawing as the note keeps it: the usual height, and a view fitted to every stroke in a
 * column `width` pixels wide, so the note opens on the whole drawing.
 */
export function finishedDrawing(block: Block, width: number): Block {
  const ink = inkOf(block);
  const height = INK_HEIGHT.initial;
  return inkBlock({ ...ink, height, view: fitView(ink.strokes, width > 0 ? width : NEW_WIDTH, height) }, block);
}

/**
 * The pen's drawing surface. A finger or the mouse draws from the start (two fingers pan and
 * zoom), and a stylus draws as it always does. Done hands over the drawing, and is disabled
 * while nothing is drawn; Discard drops it.
 */
export function PenSurface(props: { onDone: (drawing: Block) => void; onDiscard: () => void }) {
  const [block, setBlock] = createSignal<Block>(emptyDrawing());
  const drawn = () => inkOf(block()).strokes.length > 0;
  let stage!: HTMLDivElement;

  return (
    <div class="capture-pen" role="dialog" aria-label="Pen" data-testid="capture-pen">
      <header class="capture-pen-head">
        <h3>Draw</h3>
        <span class="capture-spacer" />
        <button type="button" onClick={props.onDiscard}>
          Discard
        </button>
        <button type="button" class="primary" disabled={!drawn()} onClick={() => props.onDone(finishedDrawing(block(), stage.clientWidth))}>
          Done
        </button>
      </header>
      <div class="capture-pen-stage" ref={stage}>
        <InkSurface block={block()} label="Drawing" onChange={setBlock} fill touchDraws />
      </div>
    </div>
  );
}

const theme = () => (document.documentElement.dataset.theme === "dark" ? "dark" : "light");

/** A small picture of a drawing: every stroke, fitted to the box. */
export function InkThumb(props: { block: Block; label: string }) {
  const ink = () => inkOf(props.block);
  const box = () => {
    const b = drawingBounds(ink().strokes) ?? { x0: 0, y0: 0, x1: 1, y1: 1 };
    const m = 8;
    return `${b.x0 - m} ${b.y0 - m} ${b.x1 - b.x0 + 2 * m} ${b.y1 - b.y0 + 2 * m}`;
  };
  return (
    <svg class="capture-drawing-thumb" role="img" aria-label={props.label} viewBox={box()} preserveAspectRatio="xMidYMid meet">
      <For each={paintOrder(ink().strokes)}>{({ stroke }) => <path class={`ink-${stroke.tool}`} d={cachedPath(stroke)} fill={renderColour(stroke.colour, stroke.tool, theme())} />}</For>
    </svg>
  );
}
