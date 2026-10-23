import { For, type JSX, Match, Show, Switch, createMemo, createSignal, useContext } from "solid-js";
import { parseAgentCall } from "../agentCalls";
import { type InlineNode, type Mark, parseInline } from "../editor/markup";
import { mentionText, noteArchived } from "../editor/titles";
import { followPlaceholder, resolvePlaceholder } from "../placeholders";
import { useThisNote } from "../views/thisNote";
import { wantsNewTab } from "../workspace/events";
import { AgentCallChip } from "./AgentCallChip";
import { FieldRefs } from "./fieldRefs";
import { PlaceholderLinkView, PlaceholdersInert } from "./PlaceholderLink";

/** Only these link targets render as links; anything else shows as plain text. */
const SAFE_HREF = /^(https?:|mailto:|\/|#)/i;
/** Link targets inside the app: a path on this site, or a place on this page. */
const IN_APP_HREF = /^(\/(?!\/)|#)/;

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
    case "link": {
      // A call of an agent shows as its chip (agents G19).
      const call = parseAgentCall(mark.attrs.href);
      if (call)
        return (
          <AgentCallChip agent={call.agent} call={call.call}>
            {child}
          </AgentCallChip>
        );
      if (!SAFE_HREF.test(mark.attrs.href)) return child;
      // A link into the app opens in the pane, as the workspace opens every in-app link;
      // another site opens in a new browser tab.
      return IN_APP_HREF.test(mark.attrs.href) ? (
        <a href={mark.attrs.href}>{child}</a>
      ) : (
        <a href={mark.attrs.href} target="_blank" rel="noopener noreferrer">
          {child}
        </a>
      );
    }
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
        <Match when={n.type === "placeholderLink" && n}>
          {(p) => <Placeholder noteType={p().attrs.noteType} title={p().attrs.title} />}
        </Match>
        <Match when={n.type === "mention" && n}>
          {(m) => (
            <a
              class="mention"
              classList={{ archived: noteArchived(m().attrs.id) === true }}
              href={`/notes/${m().attrs.id}`}
              data-testid="mention"
              data-id={m().attrs.id}
              title={noteArchived(m().attrs.id) ? "Archived" : undefined}
            >
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
 * A placeholder link outside an editor. Clicking it creates the note, rewrites the placeholder
 * in the note that holds it (see placeholders.ts) and opens the new note. It only shows where
 * there is no such note to rewrite, or where nothing may change: a template's fixed blocks, a
 * version from history, the conflict view.
 */
function Placeholder(props: { noteType: string; title: string }) {
  const host = useThisNote();
  const inert = useContext(PlaceholdersInert) || useContext(FieldRefs) !== undefined || !host;
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  async function follow(el: Element, background: boolean) {
    if (inert || busy() || !host) return;
    setBusy(true);
    setError(null);
    try {
      await followPlaceholder({
        host,
        noteType: props.noteType,
        title: props.title,
        from: el,
        background,
        replace: (id) => resolvePlaceholder(host, props.noteType, props.title, id),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <PlaceholderLinkView
      noteType={props.noteType}
      title={props.title}
      clickable={!inert}
      busy={busy()}
      error={error()}
      onMouseDown={(e) => {
        // A middle press would start the browser's autoscroll.
        if (!inert && e.button === 1) e.preventDefault();
      }}
      onClick={(e) => {
        if (inert || e.button !== 0 || e.altKey || e.shiftKey || e.detail > 1) return;
        e.preventDefault();
        e.stopPropagation();
        void follow(e.currentTarget, wantsNewTab(e));
      }}
      onAuxClick={(e) => {
        if (inert || e.button !== 1) return;
        e.preventDefault();
        void follow(e.currentTarget, true);
      }}
    />
  );
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
