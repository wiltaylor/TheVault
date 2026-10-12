import { For, Match, Show, Switch } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Block, Json } from "../api";
import { QueryView } from "../views/QueryView";
import { CodeBlock } from "../blocks/CodeBlock";
import { text } from "../blocks/fields";
import { AssetBlock, ImageBlock } from "../blocks/MediaBlocks";
import { MermaidBlock } from "../blocks/MermaidBlock";
import { TerminalBlock } from "../blocks/TerminalBlock";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { InkSurface } from "../ink/InkSurface";
import { Inline } from "./Inline";

/** A field value as plain text. Lists join with commas and records list their members. */
export function formatValue(value: Json | undefined): string {
  if (value === undefined || value === null) return "";
  if (Array.isArray(value)) return value.map(formatValue).join(", ");
  if (typeof value === "object") {
    return Object.entries(value)
      .map(([k, v]) => `${k}: ${formatValue(v)}`)
      .join(", ");
  }
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return String(value);
}

function ListItems(props: { items: Block[]; style: string }) {
  return (
    <For each={props.items.filter((b) => b.kind === "item")}>
      {(item) => (
        <li>
          <Show when={props.style === "todo"}>
            <input type="checkbox" checked={item.fields?.checked === true} disabled />{" "}
          </Show>
          <Inline text={text(item, "text")} />
          <Show when={(item.blocks ?? []).length > 0}>
            <Dynamic component={props.style === "numbered" ? "ol" : "ul"}>
              <ListItems items={item.blocks ?? []} style={props.style} />
            </Dynamic>
          </Show>
        </li>
      )}
    </For>
  );
}

/**
 * A content block with its text's inline markup rendered. With `onChange`, the media, technical
 * ink and canvas blocks are edited in place and report each change as the whole new block.
 */
export function BlockView(props: { block: Block; onChange?: (block: Block) => void }) {
  const b = () => props.block;
  return (
    <Switch fallback={<div class="block-placeholder">{b().kind} block</div>}>
      <Match when={b().kind === "heading"}>
        <Dynamic component={`h${Math.min(3, Math.max(1, Number(b().fields?.level ?? 2))) + 1}`}>
          <Inline text={text(b(), "text")} />
        </Dynamic>
      </Match>
      <Match when={b().kind === "paragraph"}>
        <p class="block-paragraph">
          <Inline text={text(b(), "text")} />
        </p>
      </Match>
      <Match when={b().kind === "quote"}>
        <blockquote>
          <Inline text={text(b(), "text")} />
        </blockquote>
      </Match>
      <Match when={b().kind === "list"}>
        <Dynamic component={text(b(), "style") === "numbered" ? "ol" : "ul"}>
          <ListItems items={b().blocks ?? []} style={text(b(), "style")} />
        </Dynamic>
      </Match>
      <Match when={b().kind === "code"}>
        <CodeBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "query_view"}>
        <QueryView block={b()} />
      </Match>
      <Match when={b().kind === "terminal"}>
        <TerminalBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "mermaid"}>
        <MermaidBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "image"}>
        <ImageBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "asset"}>
        <AssetBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "ink"}>
        <InkSurface block={b()} label="Drawing" onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "canvas"}>
        <CanvasSurface block={b()} label="Canvas" onChange={props.onChange} />
      </Match>
    </Switch>
  );
}

export function Blocks(props: { blocks: Block[] }) {
  return <For each={props.blocks}>{(block) => <BlockView block={block} />}</For>;
}
