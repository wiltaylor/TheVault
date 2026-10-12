---
id: "03"
kind: research
---
# Frontend library options

## Question

Which libraries should TheVault (SolidJS frontend, Rust backend) use for maps, the block editor, ink, canvas, Mermaid, code blocks, calendar/timeline, and (optionally) real-time collaboration? For each: recommendation, licence, maintenance status, Solid compatibility. Facts checked 2026-10-12 against GitHub, npm, crates.io, and official docs.

## Answer

### 1. Maps (Leaflet replacement)

**Leaflet status: not dead, but stalled.** The repo still gets commits (last push 2026-10-11), but the last stable release is 1.9.4 (2023-05-18). Leaflet 2.0 has **not** shipped: 2.0.0-alpha (2025-05-18) and 2.0.0-alpha.1 (2025-08-16) are the only releases. The November 2025 target was dropped and issue #9869 now reads "release date ... unknown". 1.x is in maintenance mode (critical fixes only). Licence BSD-2-Clause.
- https://github.com/Leaflet/Leaflet/releases
- https://github.com/Leaflet/Leaflet/issues/9869
- https://leafletjs.com/2025/05/18/leaflet-2.0.0-alpha.html

| Option | Licence | Latest release | Image maps (simple CRS) | Notes |
|---|---|---|---|---|
| OpenLayers `ol` | BSD-2-Clause | 10.11.0, 2026-10-04 | **Yes, first-class**: `Projection({units:'pixels', extent})` + `ImageStatic` source | Canvas/WebGL; heavier API; vector tiles via `ol-mapbox-style` |
| MapLibre GL JS | BSD-3-Clause | 6.14.0, 2026-10-11 | **Not in a release yet.** `addProjection` + built-in `identity` projection is merged to `main` (CHANGELOG "main" section, issue #168), not in 6.14.0. Today: fake lng/lat workaround | WebGL vector tiles, best for OSM-style basemaps |
| Leaflet 1.9.4 | BSD-2-Clause | 2023-05-18 | Yes (`L.CRS.Simple`) | Works, but 2.0 limbo plus a 1.x plugin ecosystem at risk |

**Recommendation: OpenLayers.** It is the only maintained option that handles both geographic tiles and pixel-space image maps with pins in a released version today. Revisit MapLibre once the `identity` projection ships in a 6.x release, because its vector-tile rendering is better. Solid: both are vanilla JS. Mount in `onMount` and destroy in `onCleanup`. Render pins as OL `Overlay` elements (Solid can render into them) or as vector features. `@solid-primitives/map` is unrelated (it is a reactive Map collection).
- https://openlayers.org/en/latest/examples/static-image.html
- https://github.com/maplibre/maplibre-gl-js/blob/main/CHANGELOG.md
- https://github.com/maplibre/maplibre-gl-js/issues/168

**Free tiles:** OpenFreeMap: free public instance, no API key, no view limits, commercial use allowed, MIT code. Data is OSM under ODbL, and the attribution "OpenFreeMap © OpenMapTiles Data from OpenStreetMap" is required. It is self-hostable and has no SLA. Protomaps PMTiles: a single-file tile archive served from any static host or object store via HTTP range requests (`pmtiles` npm 4.5.0, BSD-3-Clause). Good fit for self-hosting next to the Rust backend. Both serve vector tiles, which OL renders through `ol-mapbox-style`.
- https://openfreemap.org/
- https://github.com/protomaps/PMTiles

### 2. WYSIWYG block editor

| Option | Licence | Latest | Solid fit |
|---|---|---|---|
| ProseMirror | MIT | prosemirror-view 1.42.6 (npm, 2026-09) | Framework-agnostic. `NodeView` gives a DOM node, and Solid `render()` mounts into it |
| TipTap 3 (`@tiptap/core`) | MIT (core). Some Pro extensions are paid | 3.31.4, 2026-09-30 | Core is vanilla. `solid-tiptap` 0.8.0 (peer `@tiptap/core ^3`, solid ^1.7, last publish 2025-08) |
| Milkdown | MIT | 7.22.3, 2026-10-10 | Vanilla, on ProseMirror plus remark (markdown-first) |
| Lexical | MIT | 0.52.0, 2026-09-28 (pre-1.0) | Core is agnostic, but the ecosystem and `DecoratorNode` story are React-centric. No official Solid binding |
| BlockNote | MPL-2.0. `@blocknote/xl-*` is GPL-3.0 or commercial | 0.55.0, 2026-09-22 | `@blocknote/core` is on TipTap/PM, but its UI and custom-block API target React. Poor fit |

**Recommendation: TipTap 3 core** (or raw ProseMirror if more control is needed). Write a thin Solid NodeView adapter: `addNodeView` returns `{dom, contentDOM, update, destroy}`, Solid `render()` mounts the component into `dom`, and `destroy` disposes it. Use it to host query tables, kanban, maps, canvases, and ink blocks. Markdown round-trip: TipTap has a markdown extension, and Milkdown is the alternative if markdown fidelity is paramount. Note that the ProseMirror GitHub mirrors (e.g. prosemirror-view) are now archived. Development continues upstream (code.haverbeke.berlin) and npm releases are current.
- https://github.com/ueberdosis/tiptap/releases
- https://tiptap.dev/docs/editor/extensions/custom-extensions/node-views/javascript
- https://github.com/lxsmnsyc/solid-tiptap
- https://github.com/TypeCellOS/BlockNote (README licence section)
- https://github.com/facebook/lexical/releases
- https://github.com/Milkdown/milkdown/releases

### 3. Ink / drawing

- **perfect-freehand**: MIT. 1.2.3, 2026-02-01. Small, stable, pure function (points → outline polygon). Framework-free. **Recommended** as the stroke engine inside a custom Solid SVG/canvas ink block. Store raw points (with pressure from Pointer Events) and render the outline. https://github.com/steveruizok/perfect-freehand
- **tldraw**: v5.5.2, 2026-10-02. React. **Proprietary "tldraw license"**: dev/test use only. Production use requires a licence key, and the licence forbids interfering with key enforcement. Commercial licences come with a 100-day trial and value-based pricing ("talk to sales"). A free *hobby licence* exists for non-commercial use on application. **Avoid** unless TheVault is non-commercial and accepts React islands. https://github.com/tldraw/tldraw/blob/main/LICENSE.md and https://tldraw.dev/pricing
- **Excalidraw**: MIT. `@excalidraw/excalidraw` 0.18.1, 2026-04-21 (slow release cadence, active repo). React-only component. Usable only as a React island, which is not recommended for the core editor. https://github.com/excalidraw/excalidraw

### 4. Canvas (JSON Canvas)

**JSON Canvas spec 1.0** (2024-03-11, MIT; the only version, and the repo's 2026 commits only add app listings). Top level: `nodes[]` and `edges[]`. Node types are `text` (markdown), `file` (path + optional `subpath`), `link` (url), and `group` (label, background image, backgroundStyle). Every node has `id, type, x, y, width, height, color?`. Edges have `fromNode/toNode`, `fromSide/toSide`, `fromEnd/toEnd` (none/arrow), `color`, and `label`. Colours are hex or presets "1"–"6", with preset values left to the app. It does not cover ink, z-order beyond array order, or viewport state.
- https://jsoncanvas.org/spec/1.0/
- https://github.com/obsidianmd/jsoncanvas

Solid renderers: **`@dschz/solid-flow`** (MIT, a port of xyflow's Svelte Flow onto `@xyflow/system`). Stable 0.2.6 targets Solid 1.x. `1.0.0-next.22` requires Solid **2.0 RC** (solid-js latest is 1.9.17, `next` is 2.0.0-rc.14). It is a single-maintainer project, so treat it as moderate risk. The upstream xyflow (MIT) is very active, with `@xyflow/system` 0.0.83 on 2026-09-24. The old `solid-flow` npm package (2022) is dead. **Recommendation:** a custom canvas (CSS-transform pan/zoom, absolutely-positioned Solid node components, SVG edges) mapped 1:1 to JSON Canvas. The spec is small enough that a library adds little. Keep solid-flow as a fallback.
- https://github.com/dsnchz/solid-flow
- https://github.com/xyflow/xyflow

### 5. Mermaid

Current major is **12** (12.0.0 on 2026-09-10; 12.1.0 on 2026-10-02). MIT. v12 breaking change: it requires ES2024 / Safari 17.4+ / Node 22.12+. Rendering in the browser: lazy-load `mermaid`, call `mermaid.initialize({startOnLoad:false, securityLevel:'strict'})`, then `const {svg} = await mermaid.render(id, source)` inside a Solid resource/effect for a code block whose language is `mermaid`. A lighter `@mermaid-js/tiny` build is also published.
- https://github.com/mermaid-js/mermaid/releases
- https://github.com/mermaid-js/mermaid/blob/develop/packages/mermaid/CHANGELOG.md

### 6. Code highlighting

| Option | Licence | Latest | Role |
|---|---|---|---|
| CodeMirror 6 | MIT | @codemirror/view 6.43.14 (2026-10-07) | A real editor (input, keybindings, language packs, Lezer highlighting) |
| Shiki | MIT | 4.5.0, 2026-10-01 | TextMate/VS Code-accurate, read-only HTML output. Large grammar/WASM payload |
| highlight.js | BSD-3-Clause | 11.12.0, 2026-08-12 | Read-only, light. Lower accuracy |

**Recommendation:** a CodeMirror 6 NodeView for **editable** code blocks inside the TipTap/PM doc (ProseMirror's author also maintains CM6, and there is an official "embedded CodeMirror" pattern). Optionally use Shiki for read-only/export rendering. The `codemirror/dev` GitHub mirror is archived, but npm releases are current.
- https://codemirror.net/examples/ and https://prosemirror.net/examples/codemirror/
- https://github.com/shikijs/shiki/releases
- https://github.com/highlightjs/highlight.js/releases

### 7. Calendar / timeline

- **FullCalendar 7** (`@fullcalendar/core` 7.1.1, 2026-10-06; 7.0.0 shipped 2026-06-19). Standard views are MIT. Timeline/resource views are premium (commercial licence). Vanilla core, with no Solid adapter (`@fullcalendar/solid` does not exist), so mount it imperatively. **Recommended for month/week/day/agenda.** https://github.com/fullcalendar/fullcalendar/releases
- **vis-timeline** 8.5.4, 2026-08-12, `Apache-2.0 OR MIT`, vanilla. **Recommended for free timeline/Gantt-like views**, which avoids FullCalendar premium. https://github.com/visjs/vis-timeline
- **Schedule-X** (`@schedule-x/calendar` 4.9.1, 2026-09-29, MIT core, some premium plugins). A modern vanilla alternative. https://github.com/schedule-x/schedule-x
- A custom Solid timeline is also feasible if the requirements stay simple.

### 8. Real-time collaboration (CRDT)

The Rust backend makes CRDTs worth noting, because every major one has a native Rust implementation the server can run, merge, and persist:
- **Yjs** (MIT) 13.6.33, 2026-09-23 (v14 in RC). **yrs** crate 0.28.0, 2026-09-17 (y-crdt, MIT). `y-prosemirror` and the TipTap Collaboration extension integrate directly. This is the most mature path for the editor. https://github.com/yjs/yjs and https://github.com/y-crdt/y-crdt
- **Loro** (MIT): Rust-native core, `loro` crate 1.16.2 (2026-09-21), `loro-crdt` JS 1.16.4 (WASM). It has movable tree/list types, which suit canvases and kanban, and `loro-prosemirror` 0.4.4. Newer and smaller ecosystem. https://github.com/loro-dev/loro
- **Automerge** (MIT): crate 0.12.0, JS 3.5.0 (2026-09-16). JSON-document model. Its ProseMirror binding is less battle-tested. https://github.com/automerge/automerge

**Recommendation:** do not build collab in v1. Choose the editor (TipTap/PM) so that collab can be added later with Yjs + yrs, and keep the document schema CRDT-friendly (stable block IDs, no derived data in the doc).

## Consequences

- **Maps: decision is OpenLayers now, with MapLibre as a watch item.** Risk: MapLibre's planar CRS support is unreleased. If it ships in 6.x it may be the better long-term engine, so wrap the map behind a small internal interface (layers, pins, CRS kind) so the engine can be swapped.
- **Tile hosting:** OpenFreeMap has no SLA. Plan for self-hosted PMTiles (served by the Rust backend or static storage) as the production path. ODbL attribution must appear in the UI.
- **Leaflet migration:** the user's belief is half-right. Leaflet is maintained but has had no stable release in over 3 years, and 2.0 is stuck in alpha with no date. Moving off it is justified.
- **Editor commitment:** TipTap/ProseMirror is the foundational choice that every custom block depends on. TheVault must build and own a Solid NodeView adapter (Solid render roots inside PM nodes, plus event/selection handling with `stopEvent`/`ignoreMutation`). Budget for this.
- **Excluded on licence grounds:** tldraw (proprietary, production licence key) and the BlockNote XL packages (GPL-3.0). Excluded on framework grounds: Excalidraw, BlockNote UI, and Lexical's React ecosystem.
- **Solid 2.0 is in RC** (2.0.0-rc.14). Some Solid libraries (solid-flow next) already require it, while others (solid-tiptap) target 1.x. Decide whether TheVault starts on Solid 1.9 or 2.0. This affects the choice of every Solid-specific dependency.
- **Ink and canvas are custom builds** (perfect-freehand + Solid; JSON Canvas renderer). That costs more effort, but there are no licence or React lock-in risks. JSON Canvas lacks ink and viewport, so any extension fields need a namespaced convention to stay Obsidian-compatible.
- **Mermaid 12 drops older browsers** (Safari < 17.4). This sets TheVault's browser support floor unless Mermaid 11 is pinned.
- **FullCalendar premium:** avoid timeline/resource views (paid) and use vis-timeline for timelines.
- **CRDT optionality:** keep collab out of v1 scope, but choosing Yjs/yrs-compatible structures now avoids a document-model migration later.
