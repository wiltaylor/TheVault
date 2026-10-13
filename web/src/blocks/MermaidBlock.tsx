// A Mermaid 12 diagram, rendered in the browser (D36). The source is edited beside a live render;
// a source that does not render shows Mermaid's error and the source itself, never a stale
// picture.

import { Show, createEffect, createSignal, on, onCleanup } from "solid-js";
import type { Block } from "../api";
import { theme } from "../theme";
import { text, withItem } from "./fields";

type Mermaid = (typeof import("mermaid"))["default"];

let loading: Promise<Mermaid> | undefined;

/** Mermaid, loaded on first use: it is large, and most pages have no diagram. */
function mermaid(): Promise<Mermaid> {
  loading ??= import("mermaid").then(({ default: m }) => {
    configure(m);
    return m;
  });
  return loading;
}

let configuredFor: string | undefined;

/** Point Mermaid at the theme the page is in, so diagrams stay readable in dark mode. */
function configure(m: Mermaid) {
  if (configuredFor === theme()) return;
  configuredFor = theme();
  m.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    suppressErrorRendering: true,
    theme: theme() === "dark" ? "dark" : "default",
  });
}

let renders = 0;

export type Rendered = { svg: string } | { error: string };

/** `source` as SVG, or the reason it does not render. */
export async function renderMermaid(source: string): Promise<Rendered> {
  try {
    const m = await mermaid();
    configure(m);
    const { svg } = await m.render(`mermaid-${++renders}`, source);
    return { svg };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * The rendered diagram, re-rendered `delay` ms after `source` last changed. With `showSource`, a
 * source that does not render is shown as text under its error.
 */
export function MermaidDiagram(props: { source: string; delay?: number; showSource?: boolean }) {
  const [result, setResult] = createSignal<Rendered | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let latest = 0;

  createEffect(
    on(
      () => [props.source, theme()] as const,
      ([source]) => {
        clearTimeout(timer);
        const run = async () => {
          const mine = ++latest;
          const r = source.trim() ? await renderMermaid(source) : null;
          if (mine === latest) setResult(r);
        };
        if (props.delay) timer = setTimeout(run, props.delay);
        else void run();
      },
    ),
  );
  onCleanup(() => clearTimeout(timer));

  const error = () => {
    const r = result();
    return r && "error" in r ? r.error : null;
  };
  const svg = () => {
    const r = result();
    return r && "svg" in r ? r.svg : null;
  };
  return (
    <>
      <Show when={error()}>
        <p class="mermaid-error" role="alert">
          {error()}
        </p>
        <Show when={props.showSource}>
          <pre class="mermaid-fallback">{props.source}</pre>
        </Show>
      </Show>
      <Show when={svg()}>{(s) => <div class="mermaid-render" innerHTML={s()} />}</Show>
    </>
  );
}

export function MermaidBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const source = () => text(props.block, "source");
  return (
    <div class="block-mermaid">
      <Show
        when={props.onChange}
        fallback={<MermaidDiagram source={source()} showSource />}
      >
        <textarea
          class="mermaid-source"
          aria-label="Mermaid source"
          spellcheck={false}
          rows={Math.max(3, source().split("\n").length)}
          value={source()}
          onInput={(e) => props.onChange?.(withItem(props.block, "source", e.currentTarget.value))}
        />
        <MermaidDiagram source={source()} delay={250} />
      </Show>
    </div>
  );
}
