// The small icons of the canvas's corner buttons and action clusters, drawn inline so they show
// at once. Strokes take the button's text colour.

import type { JSX } from "solid-js";
import { type Arrow, arrowEnds } from "./canvas";

function Svg(props: { children: JSX.Element; size?: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={props.size ?? 16}
      height={props.size ?? 16}
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

export function MagnifierIcon() {
  return (
    <Svg>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <line x1="15.5" y1="15.5" x2="21" y2="21" />
    </Svg>
  );
}

export function TrashIcon() {
  return (
    <Svg>
      <path d="M4 7h16" />
      <path d="M10 11v6M14 11v6" />
      <path d="M6 7l1 13h10l1-13" />
      <path d="M9 7V4h6v3" />
    </Svg>
  );
}

export function OpenIcon() {
  return (
    <Svg>
      <path d="M14 4h6v6" />
      <path d="M20 4l-9 9" />
      <path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
    </Svg>
  );
}

export function CaptionIcon() {
  return (
    <Svg>
      <path d="M4 7V5h12v2" />
      <path d="M10 5v12" />
      <path d="M8 17h4" />
      <path d="M15 19l5-5-2-2-5 5v2z" />
    </Svg>
  );
}

/**
 * An arrow choice drawn as a short line with its arrowheads, turned `angle` degrees so it runs
 * the way the connection runs at its midpoint, from its start towards its end.
 */
export function ArrowIcon(props: { arrow: Arrow; angle: number }) {
  const ends = () => arrowEnds(props.arrow);
  return (
    <Svg>
      <g transform={`rotate(${props.angle} 12 12)`}>
        <line x1={ends().start ? 7 : 3} y1="12" x2={ends().end ? 17 : 21} y2="12" />
        {ends().end ? <path d="M15 7l6 5-6 5z" fill="currentColor" stroke="none" /> : null}
        {ends().start ? <path d="M9 7l-6 5 6 5z" fill="currentColor" stroke="none" /> : null}
      </g>
    </Svg>
  );
}
