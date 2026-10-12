import { For, type JSX, Match, Switch, createMemo } from "solid-js";
import { type InlineNode, type Mark, parseInline } from "../editor/markup";
import { mentionText } from "../editor/titles";

/** Only these link targets render as links; anything else shows as plain text. */
const SAFE_HREF = /^(https?:|mailto:|\/|#)/i;

function wrap(mark: Mark, child: JSX.Element): JSX.Element {
  switch (mark.type) {
    case "bold":
      return <strong>{child}</strong>;
    case "italic":
      return <em>{child}</em>;
    case "underline":
      return <u>{child}</u>;
    case "strike":
      return <s>{child}</s>;
    case "code":
      return <code>{child}</code>;
    case "highlight":
      return <mark>{child}</mark>;
    case "colour":
      return (
        <span class={`colour-${mark.attrs.colour}`} data-colour={mark.attrs.colour}>
          {child}
        </span>
      );
    case "link":
      return SAFE_HREF.test(mark.attrs.href) ? (
        <a href={mark.attrs.href} target="_blank" rel="noopener noreferrer">
          {child}
        </a>
      ) : (
        child
      );
  }
}

function Node(props: { node: InlineNode }) {
  const body = (): JSX.Element => {
    const n = props.node;
    return (
      <Switch>
        <Match when={n.type === "text" && n}>{(t) => t().text}</Match>
        <Match when={n.type === "hardBreak"}>
          <br />
        </Match>
        <Match when={n.type === "mention" && n}>
          {(m) => (
            <a class="mention" href={`/notes/${m().attrs.id}`} data-testid="mention" data-id={m().attrs.id}>
              {mentionText(m().attrs.id, m().attrs.label)}
            </a>
          )}
        </Match>
      </Switch>
    );
  };
  return <>{[...(props.node.marks ?? [])].reverse().reduce<JSX.Element>((child, m) => wrap(m, child), body())}</>;
}

/**
 * Text written in inline markup (D41), rendered: marks, links, line breaks and live mentions.
 * Mentions are plain links, which the router picks up, so this also renders outside a router.
 */
export function Inline(props: { text: string }) {
  const nodes = createMemo(() => parseInline(props.text));
  return <For each={nodes()}>{(node) => <Node node={node} />}</For>;
}
