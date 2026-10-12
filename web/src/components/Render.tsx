import { For, Match, Show, Switch } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { Block, Json } from "../api";
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

function text(block: Block, item: string): string {
  const v = block.fields?.[item];
  return typeof v === "string" ? v : "";
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

/** A content block, read-only, with its text's inline markup rendered. */
export function BlockView(props: { block: Block }) {
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
      <Match when={b().kind === "code" || b().kind === "mermaid"}>
        <figure class="block-code">
          <Show when={text(b(), "caption")}>
            <figcaption>{text(b(), "caption")}</figcaption>
          </Show>
          <pre>
            <code>{text(b(), "source")}</code>
          </pre>
        </figure>
      </Match>
      <Match when={b().kind === "terminal"}>
        <pre class="block-terminal">
          <For each={(b().blocks ?? []).filter((c) => c.kind === "command")}>
            {(c) => (
              <>
                {`${text(b(), "prompt") || "$"} ${text(c, "line")}\n`}
                {text(c, "stdout") ? `${text(c, "stdout")}\n` : ""}
                <Show when={text(c, "stderr")}>
                  <span class="stderr">{`${text(c, "stderr")}\n`}</span>
                </Show>
              </>
            )}
          </For>
        </pre>
      </Match>
    </Switch>
  );
}

export function Blocks(props: { blocks: Block[] }) {
  return <For each={props.blocks}>{(block) => <BlockView block={block} />}</For>;
}
