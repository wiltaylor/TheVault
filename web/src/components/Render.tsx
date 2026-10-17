import { For, type JSX, Match, Show, Switch } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Block, FieldDef, Json } from "../api";
import { QueryView } from "../views/QueryView";
import { CodeBlock } from "../blocks/CodeBlock";
import { columnWidth, columnsOf } from "../blocks/columns";
import { text } from "../blocks/fields";
import { AssetBlock, ImageBlock } from "../blocks/MediaBlocks";
import { MermaidBlock } from "../blocks/MermaidBlock";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { ControlBlock } from "../controls/ControlView";
import { InkSurface } from "../ink/InkSurface";
import { ImageThumb } from "./ImageField";
import { Inline } from "./Inline";
import { labelled, labelledText } from "./LabelledList";

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

/**
 * A field's value as plain text, with the note's other `fields` at hand: a list with `labels`
 * shows each value after its label, "mobile +61400000020, home +61400000021".
 */
export function fieldText(f: FieldDef, value: Json | undefined, fields?: Record<string, Json | undefined>): string {
  if (f.kind === "list" && f.labels && Array.isArray(value)) {
    return labelledText(labelled(value, fields?.[f.labels]));
  }
  return formatValue(value);
}

/**
 * A field's value, read-only, for cards, tables and the note page: an image field's image as a
 * thumbnail, everything else as [`fieldText`].
 */
export function FieldValue(props: {
  field: FieldDef;
  value: Json | undefined;
  fields?: Record<string, Json | undefined>;
  size?: "small" | "card";
}) {
  return (
    <Show
      when={props.field.kind === "asset" && props.field.accept === "image" && typeof props.value === "string" && props.value}
      fallback={fieldText(props.field, props.value, props.fields)}
    >
      {(id) => <ImageThumb id={id() as string} alt={props.field.label} size={props.size} />}
    </Show>
  );
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
      <Match when={b().kind === "columns"}>
        <ColumnsView block={b()} />
      </Match>
      <Match when={b().kind === "control"}>
        <ControlBlock block={b()} />
      </Match>
    </Switch>
  );
}

/**
 * A `columns` layout, read-only: its columns side by side at their widths, stacked in order when
 * the space it has is narrow (styles.css, `@container columns`). `child` renders each block of a
 * column; by default, as `BlockView` does. A template passes its own, to render slots.
 */
export function ColumnsView(props: { block: Block; child?: (block: Block) => JSX.Element }) {
  return (
    <div class="columns-block" data-testid="columns">
      <div class="columns-row">
        <For each={columnsOf(props.block)}>
          {(col) => (
            <div class="column" data-testid="column" data-width={columnWidth(col)} style={{ "flex-grow": String(columnWidth(col)) }}>
              <div class="column-content">
                <For each={col.blocks ?? []}>{(block) => (props.child ? props.child(block) : <BlockView block={block} />)}</For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

export function Blocks(props: { blocks: Block[] }) {
  return <For each={props.blocks}>{(block) => <BlockView block={block} />}</For>;
}
