import { For, type JSX, Match, Show, Switch, createEffect, createSignal } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Block, FieldDef, Json } from "../api";
import { QueryView } from "../views/QueryView";
import { CodeBlock } from "../blocks/LazyCode";
import { columnWidth, columnsOf } from "../blocks/columns";
import { text } from "../blocks/fields";
import { admonitionLook, findTextFor, holdsFind, nestedText, readOpen, toggleLevel, writeOpen } from "../blocks/toggle";
import { AssetBlock, ImageBlock } from "../blocks/MediaBlocks";
import { MermaidBlock } from "../blocks/MermaidBlock";
import { NoteCardBlock } from "../blocks/NoteCardBlock";
import { TableBlock } from "../blocks/TableBlock";
import { PeriodCalendar } from "../planner/PeriodCalendar";
import { CanvasSurface } from "../canvas/CanvasSurface";
import { ControlBlock } from "../controls/ControlView";
import { InkSurface } from "../ink/InkSurface";
import { TypeIcon } from "./Icon";
import { ImageThumb } from "./ImageField";
import { Inline } from "./Inline";
import { labelled, labelledShape, labelledText } from "./LabelledList";
import { LinkAnchors, linkShape, linksOf, linksText } from "./LinkList";
import { optionText } from "../options";
import { noteTitle } from "../editor/titles";
import { assetName } from "./assetNames";
import { isRuleField, ruleSummary } from "../series/rrule";

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
 * A field's value as plain text: a list of labelled values shows each value after its label,
 * "mobile +61400000020, home +61400000021", and a list of links each link's name, else its URL's
 * host and path, "Docs, example.com/blog". A select shows its options' labels, a reference its
 * notes' titles and an asset its file names, in lists and dicts too; a date range its start to its
 * end; a dict each entry after its key's label. Reactive, as titles and names load.
 */
export function fieldText(f: FieldDef, value: Json | undefined): string {
  if (value === undefined || value === null) return "";
  // An event series' rule reads as its summary (event-series E6).
  if (isRuleField(f)) return ruleSummary(value);
  const shape = labelledShape(f);
  if (shape && Array.isArray(value)) return labelledText(labelled(value, shape.key));
  if (linkShape(f) && Array.isArray(value)) return linksText(linksOf(value));
  if (Array.isArray(value)) {
    const item = f.kind === "list" ? (f.item ?? null) : { ...f, multiple: false };
    // Items that are lists or records are told apart by semicolons.
    const sep = item && (item.kind === "list" || item.kind === "dict" || item.kind === "date_range") ? "; " : ", ";
    return value.map((v) => (item ? fieldText(item, v) : formatValue(v))).join(sep);
  }
  switch (f.kind) {
    case "select":
      return typeof value === "string" ? optionText(f, value) : formatValue(value);
    case "reference":
      return typeof value === "string" ? (noteTitle(value) ?? value) : formatValue(value);
    case "asset":
      return typeof value === "string" ? (assetName(value) ?? value) : formatValue(value);
    case "dict":
      return dictText(f, value);
    case "date_range":
      // Start to end, whatever order the record's members come in.
      if (typeof value === "object" && !Array.isArray(value) && value.start !== undefined && value.end !== undefined) {
        return `${formatValue(value.start)} – ${formatValue(value.end)}`;
      }
  }
  return formatValue(value);
}

/** A dict's entries as "Key: value", apart by semicolons, the declared keys by their labels and their own kinds. */
function dictText(f: FieldDef, value: Json): string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return formatValue(value);
  const values = f.values && typeof f.values === "object" ? f.values : null;
  // Declared keys first, in their order, as the editor writes them.
  const order = (k: string) => {
    const i = f.keys?.findIndex((x) => x.name === k) ?? -1;
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  return Object.entries(value)
    .sort(([a], [b]) => order(a) - order(b))
    .map(([k, v]) => {
      const key = f.keys?.find((x) => x.name === k);
      const def = key ?? values;
      const text = def ? fieldText(def, v) : formatValue(v);
      return `${key?.label || k}: ${text}`;
    })
    .join("; ");
}

/**
 * A field's value, read-only, for cards, tables and the note page: an image field's image as a
 * thumbnail, a list of links as links that open in a new tab, everything else as [`fieldText`].
 */
export function FieldValue(props: {
  field: FieldDef;
  value: Json | undefined;
  size?: "small" | "card";
}) {
  return (
    <Switch fallback={fieldText(props.field, props.value)}>
      <Match when={props.field.kind === "asset" && props.field.accept === "image" && typeof props.value === "string" && props.value}>
        {(id) => <ImageThumb id={id() as string} alt={props.field.label} size={props.size} />}
      </Match>
      <Match when={linkShape(props.field) && Array.isArray(props.value)}>
        <LinkAnchors value={props.value} />
      </Match>
    </Switch>
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
export function BlockView(
  props: {
    block: Block;
    onChange?: (block: Block) => void;
    /** The editor's selection of the block, which an image shows its Edit and Remove buttons for. */
    selected?: boolean;
    /** Delete the block from the editor's content. */
    onRemove?: () => void;
  } & BlockPlace,
) {
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
        <QueryView block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "mermaid"}>
        <MermaidBlock block={b()} onChange={props.onChange} />
      </Match>
      <Match when={b().kind === "image"}>
        <ImageBlock block={b()} onChange={props.onChange} selected={props.selected} onRemove={props.onRemove} />
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
      <Match when={b().kind === "card"}>
        <NoteCardBlock block={b()} onChange={props.onChange} selected={props.selected} onRemove={props.onRemove} />
      </Match>
      <Match when={b().kind === "table"}>
        <TableBlock block={b()} />
      </Match>
      <Match when={b().kind === "divider"}>
        <hr class="divider-block" />
      </Match>
      <Match when={b().kind === "period_calendar"}>
        <PeriodCalendar block={b()} />
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
