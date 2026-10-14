import { For, type JSX, Match, Show, Switch, createMemo, useContext } from "solid-js";
import { type InlineNode, type Mark, parseInline } from "../editor/markup";
import { mentionText } from "../editor/titles";
import { FieldRefs } from "./fieldRefs";

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
        <Match when={n.type === "fieldRef" && n}>{(r) => <FieldRef name={r().attrs.name} />}</Match>
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

/** `{{name}}`: the shown note's value of field `name`, or the reference itself, muted, when the type has no such field. */
function FieldRef(props: { name: string }) {
  const refs = useContext(FieldRefs);
  const text = () => refs?.text(props.name) ?? null;
  return (
    <Show
      when={text() !== null}
      fallback={
        <span class="field-ref unknown" data-field-ref={props.name} title="No field of this name">
          {`{{${props.name}}}`}
        </span>
      }
    >
      <span class="field-ref" classList={{ empty: text() === "" }} data-field-ref={props.name}>
        {text() || "—"}
      </span>
    </Show>
  );
}

/**
 * Text written in inline markup (D41), rendered: marks, links, line breaks and live mentions.
 * Mentions are plain links, which the router picks up, so this also renders outside a router.
 * Inside a template's fixed blocks, which provide [`FieldRefs`], `{{name}}` shows a field's value.
 */
export function Inline(props: { text: string }) {
  // Only a template's fixed blocks provide field values; there `{{name}}` is a field reference.
  const refs = useContext(FieldRefs);
  const nodes = createMemo(() => parseInline(props.text, { fieldRefs: refs !== undefined }));
  return <For each={nodes()}>{(node) => <Node node={node} />}</For>;
}
