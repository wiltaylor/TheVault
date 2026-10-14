// The icons of the ink surface's floating controls, Lucide's shapes drawn inline so they show at
// once. Strokes take the button's text colour.

import type { JSX } from "solid-js";
import type { Background } from "./ink";

function Svg(props: { children: JSX.Element }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      {props.children}
    </svg>
  );
}

export function PenIcon() {
  return (
    <Svg>
      <path d="M21.17 6.81a2.82 2.82 0 0 0-3.99-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5z" />
    </Svg>
  );
}

export function HighlighterIcon() {
  return (
    <Svg>
      <path d="m9 11-6 6v3h9l3-3" />
      <path d="m22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4" />
    </Svg>
  );
}

export function EraserIcon() {
  return (
    <Svg>
      <path d="M21 21H8a2 2 0 0 1-1.42-.59l-4-4a2 2 0 0 1 0-2.82l10-10a2 2 0 0 1 2.83 0l6 6a2 2 0 0 1 0 2.82L12.83 21" />
      <path d="m5.08 11.09 8.83 8.83" />
    </Svg>
  );
}

export function UndoIcon() {
  return (
    <Svg>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
    </Svg>
  );
}

/** A background's look: an empty sheet, ruled lines, a grid or dots. */
export function PaperIcon(props: { background: Background }) {
  return (
    <Svg>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      {props.background === "lines" ? <path d="M7 9h10M7 13h10M7 17h10" /> : null}
      {props.background === "grid" ? <path d="M3 9h18M3 15h18M9 3v18M15 3v18" /> : null}
      {props.background === "dots" ? <path d="M9 9h.01M15 9h.01M9 15h.01M15 15h.01" stroke-width="3" /> : null}
    </Svg>
  );
}

/** A stroke size as a dot of about that width. */
export function SizeIcon(props: { size: number }) {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
      <circle cx="12" cy="12" r={Math.min(9.5, 2 + Math.sqrt(props.size) * 1.75)} fill="currentColor" />
    </svg>
  );
}
