import { type JSX, createContext } from "solid-js";
import { typeDef } from "../session";
import { TypeIcon } from "./Icon";

/**
 * True where a placeholder link only shows and is never followed: a version shown from history,
 * the conflict view. A template's fixed blocks are inert too, since they belong to no one note.
 */
export const PlaceholdersInert = createContext(false);

/**
 * `[[+type|title]]`: a link to a note that does not exist yet, drawn as a dashed chip with the
 * type's icon. When `clickable`, clicking it creates the note; the caller handles the click.
 */
export function PlaceholderLinkView(props: {
  noteType: string;
  title: string;
  clickable: boolean;
  busy?: boolean;
  error?: string | null;
  ref?: (el: HTMLSpanElement) => void;
  onClick?: JSX.EventHandler<HTMLSpanElement, MouseEvent>;
  onAuxClick?: JSX.EventHandler<HTMLSpanElement, MouseEvent>;
  onMouseDown?: JSX.EventHandler<HTMLSpanElement, MouseEvent>;
}) {
  const t = () => typeDef(props.noteType);
  const label = () => t()?.label ?? props.noteType;
  const tip = () => {
    if (props.error) return props.error;
    if (!t()) return `There is no type ${props.noteType}`;
    return props.clickable ? `Click to create this ${label()}` : `A ${label()} not created yet`;
  };
  return (
    <span
      ref={props.ref}
      class="placeholder-link"
      classList={{ clickable: props.clickable, busy: !!props.busy, error: !!props.error || !t() }}
      data-placeholder-link=""
      data-testid="placeholder-link"
      data-type={props.noteType}
      data-colour={t()?.colour ?? undefined}
      role={props.clickable ? "link" : undefined}
      aria-label={`${props.title}, a ${label()} not created yet`}
      aria-busy={props.busy ? "true" : undefined}
      title={tip()}
      onClick={props.onClick}
      onAuxClick={props.onAuxClick}
      onMouseDown={props.onMouseDown}
    >
      <TypeIcon name={t()?.icon} size={13} />
      <span class="placeholder-link-title">{props.title}</span>
    </span>
  );
}
