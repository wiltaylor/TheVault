// A small Markdown renderer for the chat panel's answers: paragraphs, headings, lists, quotes,
// fenced code, and inline code, bold, italic and links. It builds elements, never HTML, so an
// answer cannot inject markup. Links to /notes/<id> are plain in-app links, which the workspace
// opens in the active pane; other sites open in a new browser tab.

import { For, type JSX, Match, Switch, createMemo } from "solid-js";

export type MdBlock =
  | { kind: "p"; text: string }
  | { kind: "h"; level: number; text: string }
  | { kind: "ul" | "ol"; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "code"; text: string };

export type MdInline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong" | "em"; children: MdInline[] }
  | { kind: "link"; href: string; children: MdInline[] };

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;

/** The blocks of `text`. An unclosed code fence runs to the end, as it does while streaming. */
export function parseBlocks(text: string): MdBlock[] {
  const out: MdBlock[] = [];
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    if (/^\s*```/.test(line)) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i]!)) code.push(lines[i++]!);
      i++;
      out.push({ kind: "code", text: code.join("\n") });
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      out.push({ kind: "h", level: h[1]!.length, text: h[2]! });
      i++;
      continue;
    }
    const list = BULLET.test(line) ? BULLET : NUMBERED.test(line) ? NUMBERED : null;
    if (list) {
      const items: string[] = [];
      while (i < lines.length && list.test(lines[i]!)) items.push(list.exec(lines[i++]!)![1]!);
      out.push({ kind: list === BULLET ? "ul" : "ol", items });
      continue;
    }
    if (/^\s*>/.test(line)) {
      const quote: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i]!)) quote.push(lines[i++]!.replace(/^\s*>\s?/, ""));
      out.push({ kind: "quote", text: quote.join(" ") });
      continue;
    }
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i]!.trim() &&
      !HEADING.test(lines[i]!) &&
      !BULLET.test(lines[i]!) &&
      !NUMBERED.test(lines[i]!) &&
      !/^\s*(```|>)/.test(lines[i]!)
    ) {
      para.push(lines[i++]!.trim());
    }
    out.push({ kind: "p", text: para.join(" ") });
  }
  return out;
}

const INLINE = /`([^`]+)`|\*\*(.+?)\*\*|__(.+?)__|\*([^*\s][^*]*?)\*|_([^_\s][^_]*?)_|\[([^\]]+)\]\(([^)\s]+)\)/g;

export function parseInline(text: string): MdInline[] {
  const out: MdInline[] = [];
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index! > at) out.push({ kind: "text", text: text.slice(at, m.index) });
    if (m[1] !== undefined) out.push({ kind: "code", text: m[1] });
    else if (m[2] !== undefined || m[3] !== undefined) out.push({ kind: "strong", children: parseInline(m[2] ?? m[3]!) });
    else if (m[4] !== undefined || m[5] !== undefined) out.push({ kind: "em", children: parseInline(m[4] ?? m[5]!) });
    else out.push({ kind: "link", href: m[7]!, children: parseInline(m[6]!) });
    at = m.index! + m[0].length;
  }
  if (at < text.length) out.push({ kind: "text", text: text.slice(at) });
  return out;
}

/** Only these link targets render as links; anything else shows as its text. */
const SAFE_HREF = /^(https?:|mailto:|\/(?!\/))/i;

function Inline(props: { nodes: MdInline[] }): JSX.Element {
  return (
    <For each={props.nodes}>
      {(n) => (
        <Switch>
          <Match when={n.kind === "text" && n}>{(t) => t().text}</Match>
          <Match when={n.kind === "code" && n}>{(c) => <code>{c().text}</code>}</Match>
          <Match when={n.kind === "strong" && n}>
            {(s) => (
              <strong>
                <Inline nodes={s().children} />
              </strong>
            )}
          </Match>
          <Match when={n.kind === "em" && n}>
            {(s) => (
              <em>
                <Inline nodes={s().children} />
              </em>
            )}
          </Match>
          <Match when={n.kind === "link" && n}>
            {(l) =>
              SAFE_HREF.test(l().href) ? (
                l().href.startsWith("/") ? (
                  <a href={l().href}>
                    <Inline nodes={l().children} />
                  </a>
                ) : (
                  <a href={l().href} target="_blank" rel="noopener noreferrer external">
                    <Inline nodes={l().children} />
                  </a>
                )
              ) : (
                <Inline nodes={l().children} />
              )
            }
          </Match>
        </Switch>
      )}
    </For>
  );
}

function Text(props: { text: string }) {
  return <Inline nodes={parseInline(props.text)} />;
}

/** `text` as Markdown. */
export function Markdown(props: { text: string }) {
  const blocks = createMemo(() => parseBlocks(props.text));
  return (
    <div class="ai-md">
      <For each={blocks()}>
        {(b) => (
          <Switch>
            <Match when={b.kind === "p" && b}>{(p) => <p>{<Text text={p().text} />}</p>}</Match>
            <Match when={b.kind === "h" && b}>
              {(h) => (
                <p class={`ai-md-h ai-md-h${Math.min(h().level, 3)}`} role="heading" aria-level={h().level}>
                  <Text text={h().text} />
                </p>
              )}
            </Match>
            <Match when={b.kind === "ul" && b}>
              {(l) => (
                <ul>
                  <For each={l().items}>{(t) => <li>{<Text text={t} />}</li>}</For>
                </ul>
              )}
            </Match>
            <Match when={b.kind === "ol" && b}>
              {(l) => (
                <ol>
                  <For each={l().items}>{(t) => <li>{<Text text={t} />}</li>}</For>
                </ol>
              )}
            </Match>
            <Match when={b.kind === "quote" && b}>
              {(q) => (
                <blockquote>
                  <Text text={q().text} />
                </blockquote>
              )}
            </Match>
            <Match when={b.kind === "code" && b}>
              {(c) => (
                <pre>
                  <code>{c().text}</code>
                </pre>
              )}
            </Match>
          </Switch>
        )}
      </For>
    </div>
  );
}
