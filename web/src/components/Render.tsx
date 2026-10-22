import { For, type JSX, Match, Show, Switch, createEffect, createSignal } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Block, FieldDef, Json } from "../api";
import { QueryView } from "../views/QueryView";
import { CodeBlock } from "../blocks/CodeBlock";
import { columnWidth, columnsOf } from "../blocks/columns";
import { text } from "../blocks/fields";
import { admonitionLook, findTextFor, holdsFind, nestedText, readOpen, toggleLevel, writeOpen } from "../blocks/toggle";
import { AssetBlock, ImageBlock } from "../blocks/MediaBlocks";
import { MermaidBlock } from "../blocks/MermaidBlock";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { ControlBlock } from "../controls/ControlView";
import { InkSurface } from "../ink/InkSurface";
import { TypeIcon } from "./Icon";
import { ImageThumb } from "./ImageField";
import { Inline } from "./Inline";
import { labelled, labelledText } from "./LabelledList";
import { optionText } from "../options";

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
  // A select shows each option's label, where it has one.
  if (f.kind === "select" && f.option_labels?.length) {
    const text = (v: Json) => (typeof v === "string" ? optionText(f, v) : formatValue(v));
    return Array.isArray(value) ? value.map(text).join(", ") : value === undefined || value === null ? "" : text(value);
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
 * Where a block sits, for the open state of a toggle (blocks/toggle.ts): `scope` names the note
 * and field, and `path` the block's place in the content, as `2` or `3.0`.
 */
export interface BlockPlace {
  scope?: string;
  path?: string;
}

/** A block's place, `0` when it was given none (a block shown on its own). */
const placeOf = (path: string | undefined): string => path ?? "0";

/** The place of a block at `indexes` under the block at `path`, or at the top without one. */
const childPlace = (path: string | undefined, ...indexes: number[]): string =>
  path === undefined ? indexes.join(".") : [path, ...indexes].join(".");

/**
 * A content block with its text's inline markup rendered. With `onChange`, the media, technical
 * ink and canvas blocks are edited in place and report each change as the whole new block.
 */
export function BlockView(props: { block: Block; onChange?: (block: Block) => void } & BlockPlace) {
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
        <ColumnsView block={b()} scope={props.scope} path={props.path} />
      </Match>
      <Match when={b().kind === "toggle"}>
        <ToggleView block={b()} scope={props.scope} path={placeOf(props.path)} />
      </Match>
      <Match when={b().kind === "admonition"}>
        <AdmonitionView block={b()} scope={props.scope} path={placeOf(props.path)} />
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
export function ColumnsView(props: { block: Block; child?: (block: Block) => JSX.Element } & BlockPlace) {
  return (
    <div class="columns-block" data-testid="columns">
      <div class="columns-row">
        <For each={columnsOf(props.block)}>
          {(col, c) => (
            <div class="column" data-testid="column" data-width={columnWidth(col)} style={{ "flex-grow": String(columnWidth(col)) }}>
              <div class="column-content">
                <For each={col.blocks ?? []}>
                  {(block, i) =>
                    props.child ? props.child(block) : <BlockView block={block} scope={props.scope} path={childPlace(props.path, c(), i())} />
                  }
                </For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

/** A chevron pointing right, turned down by CSS when its toggle is open. */
function Chevron() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

/**
 * Whether the toggle at `place` is open, remembered on this device; it opens for a search hit
 * among `blocks`, and `initial` is its state on a device that has not changed it.
 */
function createOpen(place: Required<Pick<BlockPlace, "path">> & BlockPlace, blocks: () => Block[] | undefined, initial: boolean) {
  const [open, setOpen] = createSignal(readOpen(place.scope, place.path, initial));
  createEffect(() => {
    if (holdsFind(nestedText(blocks()), findTextFor(place.scope))) setOpen(true);
  });
  const flip = () => {
    setOpen(!open());
    writeOpen(place.scope, place.path, open());
  };
  return [open, flip] as const;
}

/** A `toggle`: its summary, plain or a heading, and the blocks under it while it is open. */
export function ToggleView(props: { block: Block; path: string; scope?: string }) {
  const [open, flip] = createOpen(props, () => props.block.blocks, false);
  const level = () => toggleLevel(props.block);
  return (
    <div class="toggle-block" data-testid="toggle" data-open={open() ? "true" : "false"}>
      <button type="button" class="toggle-button" data-testid="toggle-button" aria-expanded={open()} aria-label={`${open() ? "Close" : "Open"} toggle`} onClick={flip}>
        <Chevron />
      </button>
      <div class="toggle-content">
        <Dynamic component={level() ? `h${level() + 1}` : "div"} class={`toggle-text toggle-level-${level()}`}>
          <Inline text={text(props.block, "text")} />
        </Dynamic>
        <Show when={open()}>
          <Blocks blocks={props.block.blocks ?? []} scope={props.scope} path={props.path} />
        </Show>
      </div>
    </div>
  );
}

/** An `admonition`: a box in its colour with its icon, title and blocks. */
export function AdmonitionView(props: { block: Block; path: string; scope?: string }) {
  const look = () => admonitionLook(props.block.fields);
  const [open, flip] = createOpen(props, () => props.block.blocks, true);
  const shown = () => !look().collapsible || open();
  return (
    <div
      class="admonition"
      classList={{ collapsible: look().collapsible }}
      data-testid="admonition"
      data-kind={look().kind}
      data-colour={look().colour}
      data-icon={look().icon}
      data-open={shown() ? "true" : "false"}
      style={{ "--admonition": `var(--palette-${look().colour})` }}
    >
      <div class="admonition-icon">
        <TypeIcon name={look().icon} size={18} />
      </div>
      <Show when={look().collapsible}>
        <button type="button" class="toggle-button" data-testid="toggle-button" aria-expanded={open()} aria-label={`${open() ? "Close" : "Open"} callout`} onClick={flip}>
          <Chevron />
        </button>
      </Show>
      <div class="admonition-content">
        <div class="admonition-title">
          <Show when={look().title} fallback={look().label}>
            <Inline text={look().title} />
          </Show>
        </div>
        <Show when={shown()}>
          <Blocks blocks={props.block.blocks ?? []} scope={props.scope} path={props.path} />
        </Show>
      </div>
    </div>
  );
}

/**
 * A sequence of blocks. `scope` and `path` place them for toggles' open state: `path` is the
 * place of the block that holds them, and the top of the content has none.
 */
export function Blocks(props: { blocks: Block[] } & BlockPlace) {
  return (
    <For each={props.blocks}>
      {(block, i) => <BlockView block={block} scope={props.scope} path={childPlace(props.path, i())} />}
    </For>
  );
}
