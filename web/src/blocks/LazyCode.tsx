// CodeMirror and its languages load when the first code block or script editor shows, not with
// the app: they are a large part of what the app would otherwise load up front. Each shows
// nothing until its code has arrived. There is no Suspense boundary: one would also hold back the
// block whenever a resource inside it loads.

import { lazy } from "solid-js";

/** A note's code block, as `CodeBlock` draws it, once CodeMirror has loaded. */
export const CodeBlock = lazy(() => import("./CodeBlock").then((m) => ({ default: m.CodeBlock })));

/** A script editor, as `CodeMirror` draws it, once CodeMirror has loaded. */
export const CodeMirror = lazy(() => import("./CodeBlock").then((m) => ({ default: m.CodeMirror })));
