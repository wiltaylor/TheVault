// The note page's Outbox buttons (outbox X8, X13, X14): Reply, Reply all and Forward on an email
// or an SMS message, Propose a change on an event, and on an Outbox item itself what its status
// allows: Ready on a draft, Retry on a failed one, and Delete on both.

import { useNavigate } from "@solidjs/router";
import { Show, createResource, createSignal } from "solid-js";
import type { NoteDoc } from "../api";
import { showToast } from "../components/Toasts";
import { typeDef } from "../session";
import { errorText } from "../pages/TypeEditor";
import { conversationPeople, createDraft, googleAccount, smsReplyDrafts } from "./actions";
import {
  DRAFT_TAB_HREF,
  EMAIL,
  EVENT,
  MESSAGE,
  OUTBOX,
  type Status,
  changeDraft,
  forwardDraft,
  messageParties,
  replyAllDraft,
  replyDraft,
  smsForwardDraft,
  statusOf,
  str,
  strs,
} from "./model";

export function NoteOutboxActions(props: {
  id: string;
  type: string;
  doc: NoteDoc;
  disabled?: boolean;
  /** Set the open item's status, through the note page's own save. */
  setStatus: (status: Status, extra?: Record<string, undefined>) => void;
  /** Move the open item to the trash, as the note page's own delete does. */
  onDelete?: () => void;
}) {
  const navigate = useNavigate();
  const [busy, setBusy] = createSignal(false);
  // The Outbox is a built-in type; a vault that has it removed offers none of these.
  const hasOutbox = () => !!typeDef(OUTBOX);

  /** Create the drafts `make` builds and open the one made, or the Draft tab for several. */
  async function draft(make: () => NoteDoc[] | Promise<NoteDoc[]>) {
    setBusy(true);
    try {
      const docs = await make();
      const made = [];
      for (const doc of docs) made.push(await createDraft(doc));
      if (made.length === 1) navigate(`/notes/${made[0]!.id}`);
      else if (made.length > 1) {
        showToast(`${made.length} SMS drafts made`);
        navigate(DRAFT_TAB_HREF);
      }
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  // An email offers Reply all when it went to anyone besides its sender.
  const emailHasOthers = () => strs(props.doc.fields, "to_raw").length + strs(props.doc.fields, "cc_raw").length > 1;
  // A message offers Reply all in a conversation with several participants, or when it went to several.
  const [participants] = createResource(
    () => (props.type === MESSAGE && hasOutbox() ? str(props.doc.fields, "conversation") || null : null),
    () => conversationPeople(props.doc).catch(() => [] as string[]),
  );
  const messageHasOthers = () => {
    const { raws, people } = messageParties(props.doc, true);
    return (participants() ?? []).length > 1 || raws.length > 1 || people.length > 1;
  };

  const status = () => statusOf(props.doc.fields);
  const off = () => busy() || !!props.disabled;
  // `name` is what a screen reader hears when the label alone is ambiguous on the page: the pane's
  // history arrow is also called Forward.
  const Button = (p: { label: string; name?: string; primary?: boolean; onClick: () => void }) => (
    <button
      type="button"
      class={p.primary ? "primary outbox-note-action" : "outbox-note-action"}
      aria-label={p.name}
      disabled={off()}
      onClick={() => p.onClick()}
    >
      {p.label}
    </button>
  );
  return (
    <Show when={hasOutbox()}>
      <Show when={props.type === EMAIL}>
        <Button label="Reply" onClick={() => void draft(() => [replyDraft(props.id, props.doc)])} />
        <Show when={emailHasOthers()}>
          <Button label="Reply all" onClick={() => void draft(async () => [replyAllDraft(props.id, props.doc, await googleAccount())])} />
        </Show>
        <Button label="Forward" name="Forward this email" onClick={() => void draft(() => [forwardDraft(props.doc)])} />
      </Show>
      <Show when={props.type === MESSAGE}>
        <Button label="Reply" onClick={() => void draft(() => smsReplyDrafts(props.doc, false))} />
        <Show when={messageHasOthers()}>
          <Button label="Reply all" onClick={() => void draft(() => smsReplyDrafts(props.doc, true))} />
        </Show>
        <Button label="Forward" name="Forward this message" onClick={() => void draft(() => [smsForwardDraft(props.doc)])} />
      </Show>
      <Show when={props.type === EVENT}>
        <Button label="Propose a change" onClick={() => void draft(() => [changeDraft(props.id, props.doc)])} />
      </Show>
      <Show when={props.type === OUTBOX}>
        <Show when={status() === "draft"}>
          <Button label="Ready" primary onClick={() => props.setStatus("ready")} />
        </Show>
        <Show when={status() === "failed"}>
          <Button label="Retry" primary onClick={() => props.setStatus("ready", { error: undefined })} />
        </Show>
        <Show when={props.onDelete && (status() === "draft" || status() === "failed")}>
          <Button label="Delete" name="Delete this item" onClick={() => props.onDelete?.()} />
        </Show>
      </Show>
    </Show>
  );
}
