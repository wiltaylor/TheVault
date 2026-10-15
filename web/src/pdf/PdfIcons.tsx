// The PDF viewer's control icons: Lucide's shapes drawn inline, as the ink and canvas controls do.

import type { JSX } from "solid-js";

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

export const ChevronUpIcon = () => (
  <Svg>
    <path d="m18 15-6-6-6 6" />
  </Svg>
);

export const ChevronDownIcon = () => (
  <Svg>
    <path d="m6 9 6 6 6-6" />
  </Svg>
);

export const ZoomInIcon = () => (
  <Svg>
    <circle cx="11" cy="11" r="8" />
    <path d="m21 21-4.3-4.3M11 8v6M8 11h6" />
  </Svg>
);

export const ZoomOutIcon = () => (
  <Svg>
    <circle cx="11" cy="11" r="8" />
    <path d="m21 21-4.3-4.3M8 11h6" />
  </Svg>
);

export const FitWidthIcon = () => (
  <Svg>
    <path d="M21 3H3M21 21H3M8 12H2M22 12h-6M5 9l-3 3 3 3M19 9l3 3-3 3" />
  </Svg>
);

export const SearchIcon = () => (
  <Svg>
    <circle cx="11" cy="11" r="8" />
    <path d="m21 21-4.3-4.3" />
  </Svg>
);

export const DownloadIcon = () => (
  <Svg>
    <path d="M12 15V3M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5" />
  </Svg>
);

export const ExternalIcon = () => (
  <Svg>
    <path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
  </Svg>
);

export const MaximiseIcon = () => (
  <Svg>
    <path d="M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3" />
  </Svg>
);

export const MinimiseIcon = () => (
  <Svg>
    <path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3" />
  </Svg>
);

export const CloseIcon = () => (
  <Svg>
    <path d="M18 6 6 18M6 6l12 12" />
  </Svg>
);

/** A file card: the inline viewer folded back to its download card. */
export const CardIcon = () => (
  <Svg>
    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
    <path d="M14 2v4a2 2 0 0 0 2 2h4" />
  </Svg>
);

/** A PDF file, for a thumbnail still loading or that failed. */
export const PdfFileIcon = (props: { size?: number }) => (
  <Svg size={props.size}>
    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
    <path d="M14 2v4a2 2 0 0 0 2 2h4M9 13h1.5a1.5 1.5 0 0 1 0 3H9v-5M9 16v2" />
  </Svg>
);
