import { createSignal } from "solid-js";
import { PlaceholderLinkView } from "../components/PlaceholderLink";
import { followPlaceholder } from "../placeholders";
import { useThisNote } from "../views/thisNote";
import type { SolidNodeViewProps } from "./solidNodeView";

/** What following each placeholder chip in an editor does, for the editor's click handling. */
export const placeholderActions = new WeakMap<Element, (background: boolean) => void>();

/**
 * A placeholder link inside the editor. The editor's `FollowLinks` sends a click on it here:
 * the note is created and this node becomes a mention of it, which the note's autosave saves.
 */
export function PlaceholderChip(props: SolidNodeViewProps) {
  const host = useThisNote();
  const noteType = () => String(props.node().attrs.noteType ?? "");
  const title = () => String(props.node().attrs.title ?? "");
  // In a template's editor the text belongs to every note of the type, not one: it only shows.
  const clickable = props.editor.isEditable && !props.editor.schema.nodes.templateSlot;
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function follow(el: Element, background: boolean) {
    if (busy() || !clickable) return;
    setBusy(true);
    setError(null);
    try {
      await followPlaceholder({
        host,
        noteType: noteType(),
        title: title(),
        from: el,
        background,
        replace: (id) => {
          const mention = props.editor.schema.nodes.mention;
          if (!mention || props.getPos() === undefined) return false;
          props.replaceWith(mention.create({ id, label: null }));
          return true;
        },
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <PlaceholderLinkView
      ref={(el) => placeholderActions.set(el, (background) => void follow(el, background))}
      noteType={noteType()}
      title={title()}
      clickable={clickable}
      busy={busy()}
      error={error()}
    />
  );
}
